use crate::domain::{Confidence, Edit, Field, Issue, Track};
use std::collections::{BTreeMap, BTreeSet};

fn normalized(value: Option<&str>) -> String {
    value.unwrap_or("<unknown>").trim().to_lowercase()
}

fn release_folder(track: &Track) -> String {
    let parent = track.snapshot.path.parent();
    let Some(parent) = parent else {
        return String::new();
    };
    let name = parent
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let suffix = name
        .strip_prefix("disc")
        .or_else(|| name.strip_prefix("disk"))
        .or_else(|| name.strip_prefix("cd"));
    if track.metadata.disc.is_some() && suffix.is_some_and(|s| s.trim().parse::<u32>().is_ok()) {
        parent
            .parent()
            .unwrap_or(parent)
            .to_string_lossy()
            .to_string()
    } else {
        parent.to_string_lossy().to_string()
    }
}

pub fn album_key(track: &Track) -> String {
    let meta = &track.metadata;
    let artist = meta
        .album_artist
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or(meta.artist.as_deref());
    let parent = release_folder(track);
    format!(
        "{}|{}|{}|{}",
        normalized(artist),
        normalized(meta.album.as_deref()),
        normalized(meta.release_id.as_deref()),
        normalized(Some(&parent))
    )
}

pub fn echo_projection_key(track: &Track) -> String {
    format!(
        "{}|{}",
        normalized(
            track
                .metadata
                .album_artist
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .or(track.metadata.artist.as_deref())
        ),
        normalized(track.metadata.album.as_deref())
    )
}

fn issue(
    track: &Track,
    id: &str,
    description: String,
    confidence: Confidence,
    suggestion: Option<Edit>,
) -> Issue {
    Issue {
        rule_id: id.into(),
        track_id: track.id,
        path: track.snapshot.path.clone(),
        description,
        confidence,
        suggestion,
        verification:
            "Compare this album on a specific Echo Mini firmware and rescan after changing a copy."
                .into(),
    }
}

pub fn inspect(tracks: &[Track], echo_mini: bool) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut cohorts: BTreeMap<(String, String, String, String), Vec<&Track>> = BTreeMap::new();
    for track in tracks {
        cohorts
            .entry((
                normalized(Some(&release_folder(track))),
                normalized(track.metadata.album.as_deref()),
                normalized(track.metadata.release_id.as_deref()),
                normalized(track.metadata.date.as_deref()),
            ))
            .or_default()
            .push(track);
        for diagnostic in &track.diagnostics {
            issues.push(issue(
                track,
                "tag_conflict",
                diagnostic.clone(),
                Confidence::Observed,
                None,
            ));
        }
        if track.metadata.album.is_some() && track.metadata.album_artist.is_none() {
            issues.push(issue(
                track,
                "missing_album_artist",
                "ALBUMARTIST is missing; grouping may use track ARTIST".into(),
                Confidence::Likely,
                None,
            ));
        }
        if track.metadata.artists.len() > 1 && echo_mini {
            issues.push(issue(
                track,
                "echo_multi_artist",
                "Multiple ARTISTS values may be parsed inconsistently by Echo Mini".into(),
                Confidence::Experimental,
                None,
            ));
        }
        if track.metadata.artist.as_ref().is_some_and(|artist| {
            !track.metadata.artists.is_empty()
                && !track.metadata.artists.iter().any(|a| a == artist)
        }) {
            issues.push(issue(
                track,
                "artist_conflict",
                "ARTIST and ARTISTS disagree".into(),
                Confidence::Observed,
                None,
            ));
        }
        if echo_mini
            && [track.metadata.track.as_ref(), track.metadata.disc.as_ref()]
                .into_iter()
                .flatten()
                .any(|n| n.total.is_some())
        {
            issues.push(issue(
                track,
                "echo_number_pair",
                "Track/disc total is stored; Echo Mini parsing of n/total is unverified".into(),
                Confidence::Experimental,
                None,
            ));
        }
        if echo_mini && track.metadata.artwork_count > 0 {
            issues.push(issue(
                track,
                "echo_artwork",
                "Embedded artwork compatibility depends on image size and firmware".into(),
                Confidence::Experimental,
                None,
            ));
        }
        if !track.writable {
            issues.push(issue(
                track,
                "read_only",
                track.write_reason.clone(),
                Confidence::Observed,
                None,
            ));
        }
    }
    for cohort in cohorts.values() {
        if cohort.len() < 2 {
            continue;
        }
        let artists: BTreeSet<_> = cohort
            .iter()
            .filter_map(|t| t.metadata.album_artist.as_deref())
            .collect();
        if artists.len() > 1 {
            for track in cohort {
                issues.push(issue(track, "album_artist_conflict", "Tracks in this folder/album have different ALBUMARTIST values; confirm edition before merging".into(),
                    Confidence::Observed, None));
            }
        } else if let Some(&common) = artists.iter().next() {
            for track in cohort {
                if track.metadata.album_artist.is_none() {
                    issues.push(issue(
                        track,
                        "album_artist_candidate",
                        format!("Other tracks in this folder/album use ALBUMARTIST={common}"),
                        Confidence::Likely,
                        Some(Edit {
                            field: Field::AlbumArtist,
                            value: common.to_string(),
                        }),
                    ));
                }
            }
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AudioFormat, Metadata, NumberPair, Snapshot, TrackId};
    use std::path::PathBuf;

    fn track(id: i64, path: &str, album_artist: Option<&str>) -> Track {
        Track {
            id: TrackId::indexed(id).expect("indexed fixture"),
            snapshot: Snapshot {
                path: PathBuf::from(path),
                size: 0,
                modified_ns: 0,
                sha256: None,
            },
            format: AudioFormat::Flac,
            raw: vec![],
            diagnostics: vec![],
            writable: true,
            write_reason: String::new(),
            metadata: Metadata {
                artist: Some("Soloist".into()),
                album_artist: album_artist.map(str::to_owned),
                album: Some("Shared name".into()),
                ..Metadata::default()
            },
        }
    }

    #[test]
    fn grouping_separates_folders_and_release_artists() {
        assert_ne!(
            album_key(&track(1, "/a/one.flac", Some("A"))),
            album_key(&track(2, "/b/two.flac", Some("A")))
        );
        assert_ne!(
            album_key(&track(1, "/a/one.flac", Some("A"))),
            album_key(&track(2, "/a/two.flac", Some("B")))
        );
    }

    #[test]
    fn disc_subfolders_are_one_album_when_disc_numbers_exist() {
        let mut first = track(1, "/release/CD1/one.flac", Some("Band"));
        first.metadata.disc = Some(NumberPair {
            number: 1,
            total: Some(2),
        });
        let mut second = track(2, "/release/CD2/two.flac", Some("Band"));
        second.metadata.disc = Some(NumberPair {
            number: 2,
            total: Some(2),
        });
        assert_eq!(album_key(&first), album_key(&second));
    }

    #[test]
    fn consensus_suggests_only_missing_album_artist() {
        let tracks = [
            track(1, "/a/one.flac", Some("Various Artists")),
            track(2, "/a/two.flac", None),
        ];
        let issues = inspect(&tracks, true);
        assert!(issues.iter().any(|i| {
            i.track_id.get() == 2
                && i.suggestion
                    .as_ref()
                    .is_some_and(|e| e.value == "Various Artists")
        }));
        assert!(
            !issues
                .iter()
                .any(|i| i.track_id.get() == 1 && i.suggestion.is_some())
        );
    }

    #[test]
    fn artist_and_artists_conflict_is_observed_without_auto_delete() {
        let mut song = track(1, "/a/song.flac", Some("Band"));
        song.metadata.artists = vec!["Different person".into()];
        let issues = inspect(&[song], true);
        let conflict = issues
            .iter()
            .find(|i| i.rule_id == "artist_conflict")
            .expect("conflict");
        assert_eq!(conflict.confidence, Confidence::Observed);
        assert!(conflict.suggestion.is_none());
    }

    #[test]
    fn album_and_echo_keys_normalize_case_and_whitespace() {
        let first = track(1, "/a/one.flac", Some(" Band "));
        let mut second = track(2, "/a/two.flac", Some("band"));
        second.metadata.album = Some("shared name ".into());
        assert_eq!(album_key(&first), album_key(&second));
        assert_eq!(echo_projection_key(&first), echo_projection_key(&second));
        second.metadata.release_id = Some("different edition".into());
        assert_ne!(album_key(&first), album_key(&second));
    }

    #[test]
    fn echo_mini_issues_are_absent_from_generic_check() {
        let mut song = track(1, "/a/song.flac", Some("Band"));
        song.metadata.artists = vec!["Band".into(), "Guest".into()];
        song.metadata.track = Some(NumberPair {
            number: 1,
            total: Some(8),
        });
        song.metadata.artwork_count = 1;
        let generic = inspect(&[song.clone()], false);
        let echo = inspect(&[song], true);
        assert!(
            generic
                .iter()
                .all(|issue| !issue.rule_id.starts_with("echo_"))
        );
        for rule in ["echo_multi_artist", "echo_number_pair", "echo_artwork"] {
            assert!(echo.iter().any(|issue| issue.rule_id == rule), "{rule}");
        }
    }
}
