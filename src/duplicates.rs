#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Metadata, Snapshot, Track};
    use std::fs;
    use std::path::Path;

    fn track(id: i64, path: &Path, artist: Option<&str>, title: Option<&str>) -> Track {
        let snapshot = crate::tags::snapshot(path, false).expect("snapshot");
        Track {
            id,
            snapshot: Snapshot {
                path: path.to_path_buf(),
                ..snapshot
            },
            format: "MP3".into(),
            raw: vec![],
            metadata: Metadata {
                artist: artist.map(str::to_owned),
                title: title.map(str::to_owned),
                ..Metadata::default()
            },
            diagnostics: vec![],
            writable: true,
            write_reason: String::new(),
        }
    }

    #[test]
    fn finds_exact_and_cross_album_probable_duplicates_without_repeating_members() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths: Vec<_> = (0..4)
            .map(|i| dir.path().join(format!("{i}.mp3")))
            .collect();
        fs::write(&paths[0], b"same").expect("write");
        fs::write(&paths[1], b"same").expect("write");
        fs::write(&paths[2], b"other").expect("write");
        fs::write(&paths[3], b"other").expect("write");
        let mut tracks = vec![
            track(1, &paths[0], Some("The Band"), Some("Song")),
            track(2, &paths[1], None, None),
            track(3, &paths[2], Some(" the  band "), Some(" song ")),
            track(4, &paths[3], None, None),
        ];
        tracks[0].metadata.album = Some("Original".into());
        tracks[2].metadata.album = Some("Collection".into());
        let report = find(&tracks, |_, _| true);
        assert_eq!(report.groups.len(), 1);
        assert_eq!(report.groups[0].members.len(), 4);
        assert_eq!(report.groups[0].kind, MatchKind::Probable);
        let ids: Vec<_> = report.groups[0]
            .members
            .iter()
            .map(|m| m.track_id)
            .collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
        assert_eq!(
            report.groups[0].members[0].sha256,
            report.groups[0].members[1].sha256
        );
        assert_ne!(
            report.groups[0].members[0].sha256,
            report.groups[0].members[2].sha256
        );
    }

    #[test]
    fn missing_tags_do_not_create_probable_matches_and_cancellation_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a.mp3");
        let b = dir.path().join("b.mp3");
        fs::write(&a, b"first").expect("write");
        fs::write(&b, b"other").expect("write");
        let tracks = vec![track(1, &a, None, None), track(2, &b, None, None)];
        assert!(find(&tracks, |_, _| true).groups.is_empty());
        assert!(find(&tracks, |_, _| false).cancelled);
    }

    #[test]
    fn probable_members_have_preview_fingerprints_even_with_different_sizes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a.mp3");
        let b = dir.path().join("b.mp3");
        fs::write(&a, b"short").expect("write");
        fs::write(&b, b"longer file").expect("write");
        let tracks = vec![
            track(1, &a, Some("Band"), Some("Song")),
            track(2, &b, Some("band"), Some("song")),
        ];
        let report = find(&tracks, |_, _| true);
        assert_eq!(report.groups.len(), 1);
        assert!(
            report.groups[0]
                .members
                .iter()
                .all(|member| member.sha256.is_some())
        );
    }

    #[test]
    fn hashing_can_be_cancelled_after_candidate_discovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a.mp3");
        let b = dir.path().join("b.mp3");
        fs::write(&a, b"same").expect("write");
        fs::write(&b, b"same").expect("write");
        let tracks = vec![track(1, &a, None, None), track(2, &b, None, None)];
        let report = find(&tracks, |count, _| count <= tracks.len());
        assert!(report.cancelled);
        assert!(report.groups.is_empty());
    }

    #[test]
    fn changed_file_during_hashing_is_omitted_from_probable_groups() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a.mp3");
        let b = dir.path().join("b.mp3");
        fs::write(&a, b"first").expect("write");
        fs::write(&b, b"other").expect("write");
        let tracks = vec![
            track(1, &a, Some("Band"), Some("Song")),
            track(2, &b, Some("Band"), Some("Song")),
        ];
        let report = find(&tracks, |count, path| {
            if count == tracks.len() + 1 {
                fs::write(path, b"changed content").expect("change during hashing");
            }
            true
        });
        assert!(report.groups.is_empty());
        assert_eq!(report.errors.len(), 1);
    }
}
use crate::domain::Track;
use crate::tags;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    Exact,
    Probable,
}

#[derive(Clone, Debug, Serialize)]
pub struct DuplicateMember {
    pub track_id: i64,
    pub path: PathBuf,
    pub artist: Option<String>,
    pub title: Option<String>,
    pub album: Option<String>,
    pub disc: Option<u32>,
    pub number: Option<u32>,
    pub format: String,
    pub size: u64,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DuplicateGroup {
    pub kind: MatchKind,
    pub members: Vec<DuplicateMember>,
}

#[derive(Default, Debug, Serialize)]
pub struct DuplicateReport {
    pub groups: Vec<DuplicateGroup>,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

fn normalized(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn probable_key(track: &Track) -> Option<(String, String)> {
    let artist = normalized(track.metadata.artist.as_deref()?);
    let title = normalized(track.metadata.title.as_deref()?);
    (!artist.is_empty() && !title.is_empty()).then_some((artist, title))
}

fn root(parent: &mut [usize], mut index: usize) -> usize {
    while parent[index] != index {
        index = parent[index];
    }
    index
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let a = root(parent, a);
    let b = root(parent, b);
    parent[b] = a;
}

fn member(track: &Track, sha256: Option<String>) -> DuplicateMember {
    DuplicateMember {
        track_id: track.id,
        path: track.snapshot.path.clone(),
        artist: track.metadata.artist.clone(),
        title: track.metadata.title.clone(),
        album: track.metadata.album.clone(),
        disc: track.metadata.disc.as_ref().map(|n| n.number),
        number: track.metadata.track.as_ref().map(|n| n.number),
        format: track.format.clone(),
        size: track.snapshot.size,
        sha256,
    }
}

/// Inspect indexed tracks on demand. Files changed since the index was read are omitted.
pub fn find(tracks: &[Track], mut progress: impl FnMut(usize, &Path) -> bool) -> DuplicateReport {
    let mut report = DuplicateReport::default();
    let mut valid = Vec::new();
    for (count, track) in tracks.iter().enumerate() {
        if !progress(count + 1, &track.snapshot.path) {
            report.cancelled = true;
            return report;
        }
        match tags::snapshot(&track.snapshot.path, false) {
            Ok(current)
                if current.size == track.snapshot.size
                    && current.modified_ns == track.snapshot.modified_ns =>
            {
                valid.push(track);
            }
            Ok(_) => report.errors.push(format!(
                "{} changed since scan; refresh before checking duplicates",
                track.snapshot.path.display()
            )),
            Err(error) => report
                .errors
                .push(format!("{}: {error:#}", track.snapshot.path.display())),
        }
    }
    let mut size_counts = BTreeMap::<u64, usize>::new();
    let mut tag_counts = BTreeMap::<(String, String), usize>::new();
    for track in &valid {
        *size_counts.entry(track.snapshot.size).or_default() += 1;
        if let Some(key) = probable_key(track) {
            *tag_counts.entry(key).or_default() += 1;
        }
    }
    let mut hashes = Vec::with_capacity(valid.len());
    let mut eligible = Vec::with_capacity(valid.len());
    for (index, track) in valid.iter().enumerate() {
        let probable = probable_key(track)
            .as_ref()
            .is_some_and(|key| tag_counts[key] > 1);
        if size_counts[&track.snapshot.size] < 2 && !probable {
            hashes.push(None);
            eligible.push(true);
            continue;
        }
        if !progress(valid.len() + index + 1, &track.snapshot.path) {
            report.cancelled = true;
            return report;
        }
        match tags::hash_file(&track.snapshot.path) {
            Ok(hash) => match tags::snapshot(&track.snapshot.path, false) {
                Ok(current)
                    if current.size == track.snapshot.size
                        && current.modified_ns == track.snapshot.modified_ns =>
                {
                    hashes.push(Some(hash));
                    eligible.push(true);
                }
                _ => {
                    report.errors.push(format!(
                        "{} changed during hashing",
                        track.snapshot.path.display()
                    ));
                    hashes.push(None);
                    eligible.push(false);
                }
            },
            Err(error) => {
                report
                    .errors
                    .push(format!("{}: {error:#}", track.snapshot.path.display()));
                hashes.push(None);
                eligible.push(false);
            }
        }
    }
    let mut parents: Vec<_> = (0..valid.len()).collect();
    let mut by_tag = BTreeMap::<(String, String), usize>::new();
    let mut by_hash = BTreeMap::<String, usize>::new();
    for (index, track) in valid.iter().enumerate() {
        if !eligible[index] {
            continue;
        }
        if let Some(key) = probable_key(track) {
            if let Some(&previous) = by_tag.get(&key) {
                union(&mut parents, previous, index);
            } else {
                by_tag.insert(key, index);
            }
        }
        if let Some(hash) = &hashes[index] {
            if let Some(&previous) = by_hash.get(hash) {
                union(&mut parents, previous, index);
            } else {
                by_hash.insert(hash.clone(), index);
            }
        }
    }
    let mut grouped = BTreeMap::<usize, Vec<usize>>::new();
    for (index, &is_eligible) in eligible.iter().enumerate() {
        if is_eligible {
            grouped
                .entry(root(&mut parents, index))
                .or_default()
                .push(index);
        }
    }
    for indices in grouped.into_values().filter(|indices| indices.len() > 1) {
        let first_hash = hashes[indices[0]].as_deref();
        let kind = if first_hash.is_some()
            && indices
                .iter()
                .all(|&index| hashes[index].as_deref() == first_hash)
        {
            MatchKind::Exact
        } else {
            MatchKind::Probable
        };
        let mut members: Vec<_> = indices
            .into_iter()
            .map(|index| member(valid[index], hashes[index].clone()))
            .collect();
        members.sort_by(|a, b| a.path.cmp(&b.path));
        report.groups.push(DuplicateGroup { kind, members });
    }
    report
        .groups
        .sort_by(|a, b| a.members[0].path.cmp(&b.members[0].path));
    report
}
