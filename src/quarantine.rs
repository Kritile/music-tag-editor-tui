#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().expect("root");
        let a = root.path().join("Album/a.mp3");
        let b = root.path().join("Other/b.mp3");
        fs::create_dir_all(a.parent().expect("parent")).expect("mkdir");
        fs::create_dir_all(b.parent().expect("parent")).expect("mkdir");
        fs::write(&a, b"first").expect("write");
        fs::write(&b, b"second").expect("write");
        (root, a, b)
    }

    #[test]
    fn move_and_restore_preserve_original_bytes_and_path() {
        let (root, a, b) = setup();
        let journal = root.path().join("journals");
        let expected = crate::tags::snapshot(&a, true).expect("snapshot");
        let original_modified = fs::metadata(&a)
            .expect("metadata")
            .modified()
            .expect("modified");
        let peer = crate::tags::snapshot(&b, true).expect("snapshot");
        let record = move_file(root.path(), &journal, &expected, &peer).expect("move");
        assert!(!a.exists());
        assert_eq!(fs::read(&record.quarantined).expect("quarantine"), b"first");
        assert_eq!(
            inspect(root.path(), &journal).expect("inspect")[0].status,
            Status::Moved
        );
        restore(root.path(), &journal, &record.id).expect("restore");
        assert_eq!(fs::read(&a).expect("original"), b"first");
        assert_eq!(
            fs::metadata(&a)
                .expect("metadata")
                .modified()
                .expect("modified"),
            original_modified
        );
        assert!(!record.quarantined.exists());
    }

    #[test]
    fn move_rejects_changed_file_and_restore_rejects_occupied_path() {
        let (root, a, b) = setup();
        let journal = root.path().join("journals");
        let expected = crate::tags::snapshot(&a, true).expect("snapshot");
        let peer = crate::tags::snapshot(&b, true).expect("snapshot");
        fs::write(&a, b"changed").expect("change");
        assert!(move_file(root.path(), &journal, &expected, &peer).is_err());
        fs::write(&a, b"first").expect("reset");
        let expected = crate::tags::snapshot(&a, true).expect("snapshot");
        let record = move_file(root.path(), &journal, &expected, &peer).expect("move");
        fs::write(&a, b"new file").expect("occupied");
        assert!(restore(root.path(), &journal, &record.id).is_err());
        assert_eq!(fs::read(&a).expect("occupied"), b"new file");
        assert!(record.quarantined.exists());
    }

    #[test]
    fn restore_rejects_occupied_path_even_when_bytes_match() {
        let (root, a, b) = setup();
        let journal = root.path().join("journals");
        let expected = crate::tags::snapshot(&a, true).expect("snapshot");
        let peer = crate::tags::snapshot(&b, true).expect("snapshot");
        let record = move_file(root.path(), &journal, &expected, &peer).expect("move");
        fs::copy(&record.quarantined, &a).expect("occupy with same bytes");
        assert!(restore(root.path(), &journal, &record.id).is_err());
        assert!(record.quarantined.exists());
        assert_eq!(fs::read(&a).expect("occupied"), b"first");
    }

    #[test]
    fn move_rechecks_kept_file_after_journal_preparation() {
        let (root, a, b) = setup();
        let journal = root.path().join("journals");
        let expected = crate::tags::snapshot(&a, true).expect("snapshot");
        let peer = crate::tags::snapshot(&b, true).expect("snapshot");
        let result = move_file_with_hook(root.path(), &journal, &expected, &peer, || {
            fs::remove_file(&b).expect("remove kept file");
        });
        assert!(result.is_err());
        assert!(a.exists());
    }

    #[test]
    fn inspect_recovers_move_after_journal_was_written_before_rename() {
        let (root, a, b) = setup();
        let journal = root.path().join("journals");
        let expected = crate::tags::snapshot(&a, true).expect("snapshot");
        let peer = crate::tags::snapshot(&b, true).expect("snapshot");
        let record = move_file(root.path(), &journal, &expected, &peer).expect("move");
        let mut stale = record.clone();
        stale.status = Status::Intent;
        save(&journal, &stale).expect("save stale");
        assert_eq!(
            inspect(root.path(), &journal).expect("inspect")[0].status,
            Status::Moved
        );
    }

    #[cfg(unix)]
    #[test]
    fn restore_rejects_quarantine_symlink() {
        use std::os::unix::fs::symlink;
        let (root, a, b) = setup();
        let journal = root.path().join("journals");
        let expected = crate::tags::snapshot(&a, true).expect("snapshot");
        let peer = crate::tags::snapshot(&b, true).expect("snapshot");
        let record = move_file(root.path(), &journal, &expected, &peer).expect("move");
        let outside = root.path().join("other.mp3");
        fs::copy(&record.quarantined, &outside).expect("copy");
        fs::remove_file(&record.quarantined).expect("remove");
        symlink(&outside, &record.quarantined).expect("symlink");
        assert!(restore(root.path(), &journal, &record.id).is_err());
        assert!(!a.exists());
    }
}
use crate::{domain::Snapshot, library, tags};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const FOLDER: &str = ".music-tui-quarantine";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Intent,
    Moved,
    Restoring,
    Restored,
    Conflict,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub original: PathBuf,
    pub quarantined: PathBuf,
    pub peer: PathBuf,
    pub sha256: String,
    pub status: Status,
}

pub fn journal_dir(root: &Path) -> Result<PathBuf> {
    let path = library::data_dir()?
        .join(library::root_key(root))
        .join("quarantine");
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn journal_path(dir: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit() || byte == b'-') {
        bail!("invalid quarantine id");
    }
    Ok(dir.join(format!("{id}.json")))
}

fn save(dir: &Path, record: &Record) -> Result<()> {
    fs::create_dir_all(dir)?;
    let path = journal_path(dir, &record.id)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer_pretty(&mut temp, record)?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    File::open(dir)?.sync_all()?;
    Ok(())
}

fn load(dir: &Path, id: &str) -> Result<Record> {
    Ok(serde_json::from_reader(File::open(journal_path(
        dir, id,
    )?)?)?)
}

fn verify(root: &Path, expected: &Snapshot) -> Result<()> {
    if !expected.path.starts_with(root)
        || expected.path.starts_with(root.join(FOLDER))
        || fs::symlink_metadata(&expected.path)?
            .file_type()
            .is_symlink()
        || !expected
            .path
            .canonicalize()?
            .starts_with(root.canonicalize()?)
    {
        bail!("file is outside the library or is a symlink");
    }
    if expected.sha256.is_none() || tags::snapshot(&expected.path, true)? != *expected {
        bail!("file changed since duplicate preview");
    }
    Ok(())
}

fn validate_record(root: &Path, record: &Record) -> Result<()> {
    let relative = record
        .original
        .strip_prefix(root)
        .context("original outside library")?;
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
        || record.quarantined != root.join(FOLDER).join(&record.id).join(relative)
    {
        bail!("invalid quarantine journal paths");
    }
    for path in [&record.original, &record.quarantined] {
        if let Ok(metadata) = fs::symlink_metadata(path)
            && metadata.file_type().is_symlink()
        {
            bail!("quarantine journal refers to a symlink: {}", path.display());
        }
    }
    if let Ok(parent) = record
        .quarantined
        .parent()
        .context("missing quarantine parent")?
        .canonicalize()
        && !parent.starts_with(root.join(FOLDER))
    {
        bail!("quarantine path escapes its folder");
    }
    Ok(())
}

pub fn move_file(root: &Path, dir: &Path, expected: &Snapshot, peer: &Snapshot) -> Result<Record> {
    move_file_with_hook(root, dir, expected, peer, || {})
}

fn move_file_with_hook(
    root: &Path,
    dir: &Path,
    expected: &Snapshot,
    peer: &Snapshot,
    before_rename: impl FnOnce(),
) -> Result<Record> {
    if expected.path == peer.path {
        bail!("select a distinct copy to keep");
    }
    verify(root, expected)?;
    verify(root, peer)?;
    let base = root.join(FOLDER);
    if base.exists() && fs::symlink_metadata(&base)?.file_type().is_symlink() {
        bail!("quarantine folder is a symlink");
    }
    fs::create_dir_all(&base)?;
    let id = format!(
        "{}-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        std::process::id()
    );
    let batch_dir = base.join(&id);
    fs::create_dir(&batch_dir)?;
    let relative = expected.path.strip_prefix(root)?;
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("invalid source path");
    }
    let destination = batch_dir.join(relative);
    fs::create_dir_all(destination.parent().context("missing destination parent")?)?;
    let mut record = Record {
        id,
        original: expected.path.clone(),
        quarantined: destination,
        peer: peer.path.clone(),
        sha256: expected.sha256.clone().context("missing fingerprint")?,
        status: Status::Intent,
    };
    save(dir, &record)?;
    before_rename();
    verify(root, expected)?;
    verify(root, peer)?;
    fs::rename(&record.original, &record.quarantined)?;
    File::open(record.original.parent().context("missing source parent")?)?.sync_all()?;
    File::open(
        record
            .quarantined
            .parent()
            .context("missing destination parent")?,
    )?
    .sync_all()?;
    if tags::hash_file(&record.quarantined)? != record.sha256 {
        bail!("moved file differs from preview; inspect quarantine journal");
    }
    record.status = Status::Moved;
    save(dir, &record)?;
    Ok(record)
}

fn observed_status(record: &Record) -> Status {
    let original = tags::hash_file(&record.original).ok();
    let moved = tags::hash_file(&record.quarantined).ok();
    let valid_original = original.as_deref() == Some(record.sha256.as_str());
    let valid_moved = moved.as_deref() == Some(record.sha256.as_str());
    match (
        original.is_some(),
        moved.is_some(),
        valid_original,
        valid_moved,
    ) {
        (true, false, true, _) if record.status == Status::Intent => Status::Intent,
        (true, false, true, _) => Status::Restored,
        (false, true, _, true) => Status::Moved,
        (true, true, true, true) if record.status == Status::Restoring => Status::Restoring,
        _ => Status::Conflict,
    }
}

pub fn inspect(root: &Path, dir: &Path) -> Result<Vec<Record>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let mut record: Record = serde_json::from_reader(File::open(&path)?)?;
        validate_record(root, &record)?;
        let status = observed_status(&record);
        if status != record.status {
            record.status = status;
            save(dir, &record)?;
        }
        records.push(record);
    }
    records.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(records)
}

pub fn restore(root: &Path, dir: &Path, id: &str) -> Result<()> {
    let mut record = load(dir, id)?;
    validate_record(root, &record)?;
    record.status = observed_status(&record);
    match record.status {
        Status::Restoring => {
            fs::remove_file(&record.quarantined)?;
        }
        Status::Moved => {
            if record.original.exists() {
                bail!("original path is occupied");
            }
            let parent = record
                .original
                .parent()
                .context("missing original parent")?;
            fs::create_dir_all(parent)?;
            if !parent.canonicalize()?.starts_with(root.canonicalize()?) {
                bail!("original parent is outside library");
            }
            record.status = Status::Restoring;
            save(dir, &record)?;
            match fs::hard_link(&record.quarantined, &record.original) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    bail!("original path is occupied");
                }
                Err(_) => {
                    let mut source = File::open(&record.quarantined)?;
                    let source_meta = source.metadata()?;
                    let mut destination = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&record.original)?;
                    if let Err(error) = io::copy(&mut source, &mut destination)
                        .and_then(|_| destination.set_permissions(source_meta.permissions()))
                        .and_then(|_| {
                            destination.set_times(
                                fs::FileTimes::new().set_modified(source_meta.modified()?),
                            )
                        })
                        .and_then(|_| destination.sync_all())
                    {
                        let _ = fs::remove_file(&record.original);
                        return Err(error.into());
                    }
                }
            }
            if tags::hash_file(&record.original)? != record.sha256 {
                fs::remove_file(&record.original)?;
                bail!("restored copy failed verification");
            }
            File::open(parent)?.sync_all()?;
            fs::remove_file(&record.quarantined)?;
        }
        _ => bail!("quarantine entry cannot be restored in its current state"),
    }
    File::open(
        record
            .quarantined
            .parent()
            .context("missing quarantine parent")?,
    )?
    .sync_all()?;
    record.status = Status::Restored;
    save(dir, &record)?;
    Ok(())
}
