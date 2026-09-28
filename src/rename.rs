use crate::domain::Track;
use crate::operations;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_TEMPLATE: &str = "{artist}/{album}/{disc}-{track} {title}.{ext}";

#[derive(Debug, thiserror::Error)]
pub enum RenameError {
    #[error("source outside library or symlink: {path}", path = .path.display())]
    InvalidSource { path: PathBuf },
    #[error("rename destination already occupied: {path}", path = .path.display())]
    DestinationOccupied { path: PathBuf },
    #[error("no files need renaming")]
    EmptyPlan,
    #[error("rename preview is stale: {path}", path = .path.display())]
    PreviewStale { path: PathBuf },
    #[error("rename path outside library")]
    OutsideLibrary,
    #[error("rename batch {batch_id} stopped: {reason}")]
    BatchStopped { batch_id: String, reason: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Move {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct RenamePlan {
    pub moves: Vec<Move>,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    id: String,
    moves: Vec<JournalMove>,
}

#[derive(Serialize, Deserialize)]
struct JournalMove {
    operation: Move,
    phase: Phase,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Intent,
    Copied,
    Moved,
    Restored,
}

fn journal_dir(root: &Path) -> Result<PathBuf> {
    Ok(crate::library::data_dir()?.join(crate::library::root_key(root)))
}

fn journal_path(dir: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty() || !id.chars().all(|ch| ch.is_ascii_digit() || ch == '-') {
        bail!("invalid rename batch ID");
    }
    fs::create_dir_all(dir)?;
    Ok(dir.join(format!("rename-{id}.json")))
}

fn save(dir: &Path, journal: &Journal) -> Result<()> {
    let path = journal_path(dir, &journal.id)?;
    operations::journal::write_json(&path, journal)
}

fn clean(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|ch| {
            if ch.is_control() || "<>:\"/\\|?*".contains(ch) {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.');
    if cleaned.is_empty() {
        return "Unknown".into();
    }
    let stem = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        format!("_{cleaned}")
    } else {
        cleaned.into()
    }
}

fn render(track: &Track, template: &str) -> Result<PathBuf> {
    let path = &track.snapshot.path;
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .context("source has no extension")?;
    let metadata = &track.metadata;
    let mut output = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' {
            let mut key = String::new();
            loop {
                match chars.next() {
                    Some('}') => break,
                    Some(ch) => key.push(ch),
                    None => bail!("unclosed template placeholder"),
                }
            }
            let value = match key.as_str() {
                "artist" => metadata
                    .artist
                    .as_deref()
                    .or(metadata.album_artist.as_deref())
                    .unwrap_or("Unknown Artist")
                    .to_string(),
                "albumartist" => metadata
                    .album_artist
                    .as_deref()
                    .or(metadata.artist.as_deref())
                    .unwrap_or("Unknown Artist")
                    .to_string(),
                "album" => metadata
                    .album
                    .as_deref()
                    .unwrap_or("Unknown Album")
                    .to_string(),
                "title" => metadata.title.as_deref().unwrap_or("Untitled").to_string(),
                "disc" => metadata
                    .disc
                    .as_ref()
                    .map_or(1, |number| number.number)
                    .to_string(),
                "track" => metadata
                    .track
                    .as_ref()
                    .map_or(0, |number| number.number)
                    .to_string(),
                "ext" => ext.to_ascii_lowercase(),
                _ => bail!("unknown template placeholder: {key}"),
            };
            output.push_str(&clean(&value));
        } else if ch == '}' {
            bail!("unexpected closing brace in template");
        } else {
            output.push(ch);
        }
    }
    let relative = PathBuf::from(output);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("template resolves outside library");
    }
    if !relative
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(ext))
    {
        bail!("rename template must keep the original audio extension");
    }
    Ok(relative)
}

fn hash(path: &Path) -> Result<String> {
    operations::fingerprint::hash_file(path)
}

fn create_verified_copy(source: &Path, destination: &Path, expected_hash: &str) -> Result<()> {
    let parent = destination.parent().context("missing destination parent")?;
    match fs::hard_link(source, destination) {
        Ok(()) => {
            if !operations::fingerprint::matches_hash(destination, expected_hash)? {
                fs::remove_file(destination)?;
                bail!("source changed while linking: {}", source.display());
            }
        }
        Err(link_error) => {
            if fs::symlink_metadata(destination).is_ok() {
                return Err(link_error).context("rename destination became occupied");
            }
            if !operations::preflight::has_space(parent, fs::metadata(source)?.len())? {
                bail!("insufficient destination space");
            }
            let metadata = fs::metadata(source)?;
            let temp = tempfile::NamedTempFile::new_in(parent)?;
            if !operations::transaction::copy_verified(source, temp.path(), expected_hash)? {
                bail!("copied file failed verification: {}", source.display());
            }
            temp.as_file()
                .set_times(std::fs::FileTimes::new().set_modified(metadata.modified()?))?;
            fs::set_permissions(temp.path(), metadata.permissions())?;
            temp.as_file().sync_all()?;
            operations::transaction::create_noclobber(temp, destination)?;
        }
    }
    Ok(())
}

fn reject_symlinks(root: &Path, path: &Path) -> Result<()> {
    if let Some(found) = operations::preflight::symlink_component(root, path)? {
        bail!("rename path contains symlink: {}", found.display());
    }
    Ok(())
}

pub fn plan(
    root: &Path,
    tracks: &[Track],
    template: &str,
) -> std::result::Result<RenamePlan, RenameError> {
    let root = root.canonicalize()?;
    let mut moves = Vec::new();
    let mut destinations = HashSet::new();
    let sources: HashSet<_> = tracks
        .iter()
        .map(|track| track.snapshot.path.clone())
        .collect();
    for track in tracks {
        let source = &track.snapshot.path;
        if fs::symlink_metadata(source)?.file_type().is_symlink()
            || !source.canonicalize()?.starts_with(&root)
            || operations::preflight::symlink_component(&root, source)?.is_some()
        {
            return Err(RenameError::InvalidSource {
                path: source.clone(),
            });
        }
        let destination = root.join(render(track, template)?);
        if destination == *source {
            continue;
        }
        if !destinations.insert(destination.clone())
            || sources.contains(&destination)
            || destination.exists()
        {
            return Err(RenameError::DestinationOccupied { path: destination });
        }
        reject_symlinks(&root, &destination)?;
        moves.push(Move {
            source: source.clone(),
            destination,
            sha256: hash(source)?,
        });
    }
    Ok(RenamePlan { moves })
}

pub fn run(root: &Path, plan: &RenamePlan) -> std::result::Result<String, RenameError> {
    run_in(root, plan, &journal_dir(root)?)
}

fn run_in(root: &Path, plan: &RenamePlan, dir: &Path) -> std::result::Result<String, RenameError> {
    let root = root.canonicalize()?;
    if plan.moves.is_empty() {
        return Err(RenameError::EmptyPlan);
    }
    for operation in &plan.moves {
        if !operation.source.starts_with(&root) || !operation.destination.starts_with(&root) {
            return Err(RenameError::OutsideLibrary);
        }
        reject_symlinks(&root, &operation.source)?;
        reject_symlinks(&root, &operation.destination)?;
        if operation.destination.exists() || hash(&operation.source)? != operation.sha256 {
            return Err(RenameError::PreviewStale {
                path: operation.source.clone(),
            });
        }
    }
    let id = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(anyhow::Error::from)?
            .as_nanos(),
        std::process::id()
    );
    let mut journal = Journal {
        id: id.clone(),
        moves: plan
            .moves
            .iter()
            .cloned()
            .map(|operation| JournalMove {
                operation,
                phase: Phase::Intent,
            })
            .collect(),
    };
    save(dir, &journal)?;
    for index in 0..journal.moves.len() {
        let operation = journal.moves[index].operation.clone();
        let parent = operation
            .destination
            .parent()
            .context("missing destination parent")?;
        fs::create_dir_all(parent)?;
        reject_symlinks(&root, parent)?;
        if operation.destination.exists() || hash(&operation.source)? != operation.sha256 {
            return Err(RenameError::BatchStopped {
                batch_id: id,
                reason: "source or destination changed".into(),
            });
        }
        reject_symlinks(&root, &operation.source)?;
        create_verified_copy(&operation.source, &operation.destination, &operation.sha256)
            .with_context(|| format!("rename batch {id} stopped"))?;
        operations::transaction::sync_dir(parent)?;
        journal.moves[index].phase = Phase::Copied;
        save(dir, &journal)?;
        if hash(&operation.source)? != operation.sha256 {
            return Err(RenameError::BatchStopped {
                batch_id: id,
                reason: "source changed before removal".into(),
            });
        }
        fs::remove_file(&operation.source)?;
        if let Some(source_parent) = operation.source.parent() {
            operations::transaction::sync_dir(source_parent)?;
        }
        journal.moves[index].phase = Phase::Moved;
        save(dir, &journal)?;
    }
    Ok(id)
}

pub fn undo(root: &Path, id: &str) -> Result<()> {
    undo_in(root, id, &journal_dir(root)?)
}

pub fn latest(root: &Path) -> Result<Option<String>> {
    let dir = journal_dir(root)?;
    if !dir.exists() {
        return Ok(None);
    }
    let mut ids = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str()
            && let Some(id) = name
                .strip_prefix("rename-")
                .and_then(|name| name.strip_suffix(".json"))
            && id.chars().all(|ch| ch.is_ascii_digit() || ch == '-')
        {
            let journal: Journal = serde_json::from_reader(File::open(entry.path())?)?;
            if journal
                .moves
                .iter()
                .any(|operation| operation.phase != Phase::Restored)
            {
                ids.push(id.to_string());
            }
        }
    }
    Ok(ids.into_iter().max())
}

fn undo_in(root: &Path, id: &str, dir: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let path = journal_path(dir, id)?;
    let mut journal: Journal = serde_json::from_reader(File::open(path)?)?;
    for index in (0..journal.moves.len()).rev() {
        let operation = journal.moves[index].operation.clone();
        if !operation.source.starts_with(&root) || !operation.destination.starts_with(&root) {
            bail!("journal path outside library");
        }
        reject_symlinks(&root, &operation.source)?;
        reject_symlinks(&root, &operation.destination)?;
        if operation.source.exists() {
            if hash(&operation.source)? != operation.sha256 {
                bail!(
                    "cannot restore changed original path: {}",
                    operation.source.display()
                );
            }
            if operation.destination.exists() {
                if journal.moves[index].phase != Phase::Copied {
                    bail!(
                        "cannot identify occupied destination: {}",
                        operation.destination.display()
                    );
                }
                if hash(&operation.destination)? != operation.sha256 {
                    bail!("renamed file changed: {}", operation.destination.display());
                }
                fs::remove_file(&operation.destination)?;
                if let Some(parent) = operation.destination.parent() {
                    operations::transaction::sync_dir(parent)?;
                }
            }
            journal.moves[index].phase = Phase::Restored;
            save(dir, &journal)?;
            continue;
        }
        if journal.moves[index].phase == Phase::Intent
            || journal.moves[index].phase == Phase::Restored
            || !operation.destination.exists()
            || hash(&operation.destination)? != operation.sha256
        {
            bail!(
                "renamed file changed or missing: {}",
                operation.destination.display()
            );
        }
        let parent = operation
            .source
            .parent()
            .context("missing original parent")?;
        create_verified_copy(&operation.destination, &operation.source, &operation.sha256)?;
        operations::transaction::sync_dir(parent)?;
        journal.moves[index].phase = Phase::Copied;
        save(dir, &journal)?;
        fs::remove_file(&operation.destination)?;
        if let Some(parent) = operation.source.parent() {
            operations::transaction::sync_dir(parent)?;
        }
        if let Some(parent) = operation.destination.parent() {
            operations::transaction::sync_dir(parent)?;
        }
        journal.moves[index].phase = Phase::Restored;
        save(dir, &journal)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Metadata, NumberPair};
    use std::fs;

    fn track(path: &Path) -> Track {
        let mut track = crate::tags::read_track(path).expect("track");
        track.metadata = Metadata {
            artist: Some("Band".into()),
            album: Some("Album".into()),
            title: Some("Song".into()),
            disc: Some(NumberPair {
                number: 1,
                total: None,
            }),
            track: Some(NumberPair {
                number: 2,
                total: None,
            }),
            ..Metadata::default()
        };
        track
    }

    #[test]
    fn moves_inside_library_and_undo_restores_original_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("music");
        fs::create_dir(&root).expect("root");
        let source = root.join("old.mp3");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3"),
            &source,
        )
        .expect("fixture");
        let old_time = UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        File::options()
            .write(true)
            .open(&source)
            .expect("open source")
            .set_times(std::fs::FileTimes::new().set_modified(old_time))
            .expect("set mtime");
        let before = fs::read(&source).expect("source");
        let preview = plan(&root, &[track(&source)], DEFAULT_TEMPLATE).expect("preview");
        assert_eq!(preview.moves.len(), 1);
        let id = run_in(&root, &preview, temp.path()).expect("rename");
        assert!(!source.exists());
        assert_eq!(
            fs::read(&preview.moves[0].destination).expect("moved"),
            before
        );
        assert_eq!(
            fs::metadata(&preview.moves[0].destination)
                .expect("moved metadata")
                .modified()
                .expect("mtime"),
            old_time
        );
        undo_in(&root, &id, temp.path()).expect("undo");
        assert_eq!(fs::read(&source).expect("restored"), before);
        assert_eq!(
            fs::metadata(&source)
                .expect("restored metadata")
                .modified()
                .expect("mtime"),
            old_time
        );
    }

    #[test]
    fn rejects_path_escape_and_existing_destination() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("music");
        fs::create_dir(&root).expect("root");
        let source = root.join("old.mp3");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3"),
            &source,
        )
        .expect("fixture");
        assert!(plan(&root, &[track(&source)], "../{title}.{ext}").is_err());
        assert!(plan(&root, &[track(&source)], "{artist}/{title}").is_err());
        fs::create_dir_all(root.join("Band/Album")).expect("target dir");
        fs::write(root.join("Band/Album/1-2 Song.mp3"), b"other").expect("collision");
        assert!(matches!(
            plan(&root, &[track(&source)], DEFAULT_TEMPLATE),
            Err(RenameError::DestinationOccupied { .. })
        ));
    }

    #[test]
    fn undo_removes_verified_duplicate_after_interrupted_copy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("music");
        fs::create_dir(&root).expect("root");
        let source = root.join("old.mp3");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3"),
            &source,
        )
        .expect("fixture");
        let preview = plan(&root, &[track(&source)], DEFAULT_TEMPLATE).expect("preview");
        let operation = preview.moves[0].clone();
        fs::create_dir_all(operation.destination.parent().expect("parent")).expect("target dir");
        fs::copy(&source, &operation.destination).expect("interrupted copy");
        save(
            temp.path(),
            &Journal {
                id: "123".into(),
                moves: vec![JournalMove {
                    operation: operation.clone(),
                    phase: Phase::Copied,
                }],
            },
        )
        .expect("journal");
        undo_in(&root, "123", temp.path()).expect("recover duplicate");
        assert!(source.exists());
        assert!(!operation.destination.exists());
        fs::copy(&source, &operation.destination).expect("unrelated same-byte file");
        assert!(undo_in(&root, "123", temp.path()).is_err());
        assert!(operation.destination.exists());
    }
}
