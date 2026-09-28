use crate::domain::{Edit, EditOperation, Field, FieldValue, Snapshot, Track};
use crate::{library, tags};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pending {
    pub expected: Snapshot,
    pub edits: Vec<EditOperation>,
    #[serde(default)]
    pub before: Vec<Before>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Before {
    pub field: Field,
    pub value: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Status {
    Intent,
    BackedUp,
    Written,
    Verified,
    Failed,
    Restored,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub pending: Pending,
    pub backup: PathBuf,
    pub backup_hash: Option<String>,
    pub written_hash: Option<String>,
    pub status: Status,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Batch {
    pub version: u32,
    pub id: String,
    pub entries: Vec<Entry>,
}

const FORMAT_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
struct StagingFile {
    version: u32,
    entries: Vec<Pending>,
}

#[derive(Serialize)]
struct StagingFileRef<'a> {
    version: u32,
    entries: &'a [Pending],
}

#[derive(Deserialize)]
struct LegacyPending {
    expected: Snapshot,
    edits: Vec<Edit>,
    #[serde(default)]
    before: Vec<Before>,
}

impl TryFrom<LegacyPending> for Pending {
    type Error = String;

    fn try_from(value: LegacyPending) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            expected: value.expected,
            edits: value
                .edits
                .into_iter()
                .map(EditOperation::try_from)
                .collect::<std::result::Result<_, _>>()?,
            before: value.before,
        })
    }
}

#[derive(Deserialize)]
struct LegacyEntry {
    pending: LegacyPending,
    backup: PathBuf,
    backup_hash: Option<String>,
    written_hash: Option<String>,
    status: Status,
    error: Option<String>,
}

#[derive(Deserialize)]
struct LegacyBatch {
    id: String,
    entries: Vec<LegacyEntry>,
}

fn read_staging(path: &Path) -> Result<Vec<Pending>> {
    let value: serde_json::Value = serde_json::from_reader(File::open(path)?)?;
    if value.is_array() {
        let old: Vec<LegacyPending> = serde_json::from_value(value)?;
        return old
            .into_iter()
            .map(|p| p.try_into().map_err(anyhow::Error::msg))
            .collect();
    }
    let file: StagingFile = serde_json::from_value(value)?;
    if file.version != FORMAT_VERSION {
        bail!("unsupported staging format version {}", file.version);
    }
    Ok(file.entries)
}

fn read_batch(path: &Path) -> Result<Batch> {
    let value: serde_json::Value = serde_json::from_reader(File::open(path)?)?;
    if value.get("version").is_none() {
        let old: LegacyBatch = serde_json::from_value(value)?;
        return Ok(Batch {
            version: FORMAT_VERSION,
            id: old.id,
            entries: old
                .entries
                .into_iter()
                .map(|e| {
                    Ok(Entry {
                        pending: e.pending.try_into().map_err(anyhow::Error::msg)?,
                        backup: e.backup,
                        backup_hash: e.backup_hash,
                        written_hash: e.written_hash,
                        status: e.status,
                        error: e.error,
                    })
                })
                .collect::<Result<_>>()?,
        });
    }
    let batch: Batch = serde_json::from_value(value)?;
    if batch.version != FORMAT_VERSION {
        bail!("unsupported journal format version {}", batch.version);
    }
    Ok(batch)
}

fn save_staging(root: &Path, entries: &[Pending]) -> Result<()> {
    durable_json(
        &pending_path(root)?,
        &StagingFileRef {
            version: FORMAT_VERSION,
            entries,
        },
    )
}

fn work_dir(root: &Path) -> Result<PathBuf> {
    let path = library::data_dir()?.join(library::root_key(root));
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn durable_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("missing journal parent")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn pending_path(root: &Path) -> Result<PathBuf> {
    Ok(work_dir(root)?.join("staging.json"))
}

pub fn load_staged(root: &Path) -> Result<Vec<Pending>> {
    let path = pending_path(root)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    read_staging(&path)
}

pub fn stage(root: &Path, track: &Track, edit: Edit) -> Result<Vec<Pending>> {
    stage_operation(root, track, edit.try_into().map_err(anyhow::Error::msg)?)
}

pub fn stage_operation(root: &Path, track: &Track, edit: EditOperation) -> Result<Vec<Pending>> {
    if !track.writable {
        bail!("{}", track.write_reason);
    }
    edit.validate().map_err(anyhow::Error::msg)?;
    tags::validate_operation_for_format(track.format, &edit)?;
    let field = edit.field();
    if !track.snapshot.path.starts_with(root) {
        bail!("track is outside library root");
    }
    let mut pending = load_staged(root)?;
    let current = tags::snapshot(&track.snapshot.path, true)?;
    if current.size != track.snapshot.size || current.modified_ns != track.snapshot.modified_ns {
        bail!("file changed since scan; rescan before staging");
    }
    let fresh = tags::read_track(&track.snapshot.path)?;
    let old_value = fresh.metadata.value(field);
    if let Some(item) = pending.iter_mut().find(|p| p.expected.path == current.path) {
        if item.expected != current {
            bail!("file changed since earlier staging");
        }
        if !item.before.iter().any(|b| b.field == field) {
            item.before.push(Before {
                field,
                value: old_value,
            });
        }
        if matches!(
            edit,
            EditOperation::Set { .. } | EditOperation::Clear { .. }
        ) {
            item.edits.retain(|e| e.field() != field);
        }
        item.edits.push(edit);
    } else {
        pending.push(Pending {
            expected: current,
            before: vec![Before {
                field,
                value: old_value,
            }],
            edits: vec![edit],
        });
    }
    save_staging(root, &pending)?;
    Ok(pending)
}

pub fn clear_staged(root: &Path) -> Result<()> {
    save_staging(root, &[])
}

pub fn diff(pending: &Pending) -> Vec<String> {
    pending
        .edits
        .iter()
        .map(|edit| {
            format!(
                "{}: {:?} -> {}",
                edit.field().label(),
                pending
                    .before
                    .iter()
                    .find(|b| b.field == edit.field())
                    .and_then(|b| b.value.as_deref()),
                describe_edit(edit)
            )
        })
        .collect()
}

fn describe_edit(edit: &EditOperation) -> String {
    match edit {
        EditOperation::Set { value, .. } => match value {
            FieldValue::Text(value) | FieldValue::Date(value) => format!("{value:?}"),
            FieldValue::Number(pair) => match pair.total {
                Some(total) => format!("{}/{total}", pair.number),
                None => pair.number.to_string(),
            },
            FieldValue::TextList(values) => format!("{values:?}"),
        },
        EditOperation::Clear { .. } => "<clear>".into(),
        EditOperation::AddValue { value, .. } => format!("+{value:?}"),
        EditOperation::RemoveValue { value, .. } => format!("-{value:?}"),
    }
}

pub fn summary(pending: &[Pending]) -> (usize, usize, u64) {
    (
        pending.len(),
        pending.iter().map(|p| p.edits.len()).sum(),
        pending.iter().map(|p| p.expected.size).sum(),
    )
}

fn journal_path(root: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit() || c == '-') {
        bail!("invalid batch ID");
    }
    Ok(work_dir(root)?.join(format!("batch-{id}.json")))
}

fn save(root: &Path, batch: &Batch) -> Result<()> {
    durable_json(&journal_path(root, &batch.id)?, batch)
}

fn load(root: &Path, id: &str) -> Result<Batch> {
    read_batch(&journal_path(root, id)?)
}

fn validate_entry(root: &Path, entry: &Entry) -> Result<()> {
    let target = &entry.pending.expected.path;
    if !target.starts_with(root)
        || (target.exists()
            && (fs::symlink_metadata(target)?.file_type().is_symlink()
                || !target.canonicalize()?.starts_with(root)))
    {
        bail!(
            "journal target is outside library or became a symlink: {}",
            target.display()
        );
    }
    let backup_root = work_dir(root)?;
    if !entry.backup.starts_with(&backup_root) {
        bail!(
            "journal backup is outside application data: {}",
            entry.backup.display()
        );
    }
    if entry.backup.exists()
        && (fs::symlink_metadata(&entry.backup)?
            .file_type()
            .is_symlink()
            || !entry
                .backup
                .canonicalize()?
                .starts_with(backup_root.canonicalize()?))
    {
        bail!(
            "journal backup became a symlink: {}",
            entry.backup.display()
        );
    }
    Ok(())
}

fn preflight(root: &Path, pending: &[Pending]) -> Result<()> {
    let mut paths = std::collections::HashSet::new();
    let mut required_by_parent = std::collections::HashMap::<PathBuf, u64>::new();
    let mut backup_bytes = 0u64;
    for item in pending {
        let path = &item.expected.path;
        if !paths.insert(path) {
            bail!("duplicate target: {}", path.display());
        }
        if fs::symlink_metadata(path)?.file_type().is_symlink()
            || !path.canonicalize()?.starts_with(root)
        {
            bail!(
                "target path changed or became a symlink: {}",
                path.display()
            );
        }
        let current = tags::snapshot(path, true)?;
        if current != item.expected {
            bail!("file changed since preview: {}", path.display());
        }
        let record = tags::read_track(path)?;
        if !record.writable {
            bail!("{}: {}", path.display(), record.write_reason);
        }
        for edit in &item.edits {
            tags::validate_operation_for_format(record.format, edit)?;
        }
        let parent = path.parent().context("target has no parent")?;
        let probe = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("cannot write beside {}", path.display()))?;
        drop(probe);
        *required_by_parent.entry(parent.to_path_buf()).or_default() +=
            item.expected.size.saturating_mul(2);
        backup_bytes = backup_bytes.saturating_add(item.expected.size);
    }
    if fs2::available_space(work_dir(root)?)? < backup_bytes {
        bail!("insufficient free space for full-file backups");
    }
    for (parent, required) in required_by_parent {
        if fs2::available_space(&parent)? < required {
            bail!(
                "insufficient free space for backup and temporary files in {}",
                parent.display()
            );
        }
    }
    Ok(())
}

fn new_temp(path: &Path) -> Result<tempfile::NamedTempFile> {
    let parent = path.parent().context("target has no parent")?;
    let suffix = format!(
        ".{}",
        path.extension()
            .and_then(|s| s.to_str())
            .context("target has no extension")?
    );
    Ok(tempfile::Builder::new()
        .prefix(".music-tui-")
        .suffix(&suffix)
        .tempfile_in(parent)?)
}

fn sync_parent(path: &Path) -> Result<()> {
    File::open(path.parent().context("missing parent")?)?.sync_all()?;
    Ok(())
}

pub fn apply(root: &Path) -> Result<String> {
    let pending = load_staged(root)?;
    if pending.is_empty() {
        bail!("nothing staged");
    }
    preflight(root, &pending)?;
    let id = format!(
        "{}-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        std::process::id()
    );
    let backup_dir = work_dir(root)?.join(format!("backup-{id}"));
    fs::create_dir(&backup_dir)?;
    let mut batch = Batch {
        version: FORMAT_VERSION,
        id: id.clone(),
        entries: pending
            .into_iter()
            .enumerate()
            .map(|(index, p)| Entry {
                backup: backup_dir.join(format!(
                    "{index}.{}",
                    p.expected
                        .path
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("audio")
                )),
                pending: p,
                backup_hash: None,
                written_hash: None,
                status: Status::Intent,
                error: None,
            })
            .collect(),
    };
    save(root, &batch)?;
    for index in 0..batch.entries.len() {
        if let Err(err) = apply_one(root, &mut batch, index) {
            if batch.entries[index].status != Status::Verified {
                batch.entries[index].status = Status::Failed;
            }
            batch.entries[index].error = Some(format!("{err:#}"));
            save(root, &batch)?;
            bail!(
                "batch {} stopped at {}: {err:#}; use `recover` or `undo`",
                id,
                batch.entries[index].pending.expected.path.display()
            );
        }
    }
    clear_staged(root)?;
    Ok(id)
}

fn apply_one(root: &Path, batch: &mut Batch, index: usize) -> Result<()> {
    let path = batch.entries[index].pending.expected.path.clone();
    if fs::symlink_metadata(&path)?.file_type().is_symlink() {
        bail!("target became a symlink");
    }
    if tags::snapshot(&path, true)? != batch.entries[index].pending.expected {
        bail!("file changed since preview");
    }
    let backup = batch.entries[index].backup.clone();
    fs::copy(&path, &backup)?;
    File::open(&backup)?.sync_all()?;
    let backup_hash = tags::hash_file(&backup)?;
    if backup_hash
        != batch.entries[index]
            .pending
            .expected
            .sha256
            .as_deref()
            .context("missing expected hash")?
    {
        bail!("backup verification failed");
    }
    batch.entries[index].backup_hash = Some(backup_hash);
    batch.entries[index].status = Status::BackedUp;
    save(root, batch)?;
    let temp = new_temp(&path)?;
    tags::write_to_temp(&path, temp.path(), &batch.entries[index].pending.edits)?;
    let new_hash = tags::hash_file(temp.path())?;
    if tags::hash_file(&path)?
        != batch.entries[index]
            .pending
            .expected
            .sha256
            .as_deref()
            .context("missing expected hash")?
    {
        bail!("file changed during write preparation");
    }
    batch.entries[index].written_hash = Some(new_hash.clone());
    save(root, batch)?;
    temp.as_file().sync_all()?;
    temp.persist(&path).map_err(|e| e.error)?;
    sync_parent(&path)?;
    batch.entries[index].status = Status::Written;
    save(root, batch)?;
    if tags::hash_file(&path)? != new_hash {
        bail!("post-rename verification failed");
    }
    batch.entries[index].status = Status::Verified;
    save(root, batch)?;
    library::Index::open(root)?.refresh(&path)?;
    Ok(())
}

pub fn recover(root: &Path, selected: Option<&str>) -> Result<()> {
    for line in recover_report(root, selected)? {
        println!("{line}");
    }
    Ok(())
}

pub fn latest_batch(root: &Path) -> Result<Option<String>> {
    let mut ids = Vec::new();
    for entry in fs::read_dir(work_dir(root)?)? {
        let name = entry?.file_name().to_string_lossy().to_string();
        if let Some(id) = name
            .strip_prefix("batch-")
            .and_then(|s| s.strip_suffix(".json"))
        {
            let batch = load(root, id)?;
            if batch
                .entries
                .iter()
                .any(|entry| entry.status == Status::Verified)
            {
                ids.push(id.to_string());
            }
        }
    }
    Ok(ids.into_iter().max())
}

pub fn recover_report(root: &Path, selected: Option<&str>) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    let paths = if let Some(id) = selected {
        vec![journal_path(root, id)?]
    } else {
        let mut paths = Vec::new();
        for entry in fs::read_dir(work_dir(root)?)? {
            let path = entry?.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("batch-") && n.ends_with(".json"))
            {
                paths.push(path);
            }
        }
        paths
    };
    for path in paths {
        let mut batch = read_batch(&path)?;
        let mut changed = false;
        for entry in &mut batch.entries {
            validate_entry(root, entry)?;
            if entry.status == Status::Restored {
                continue;
            }
            let current = tags::hash_file(&entry.pending.expected.path).ok();
            let original = entry.pending.expected.sha256.as_deref();
            let written = entry.written_hash.as_deref();
            let backup = tags::hash_file(&entry.backup).ok();
            let backup_valid = backup.as_deref() == original && original.is_some();
            if backup_valid && entry.backup_hash.is_none() {
                entry.backup_hash = backup;
                changed = true;
            }
            let observed = if current.as_deref() == written && written.is_some() && backup_valid {
                Status::Verified
            } else if current.as_deref() == original && backup_valid {
                Status::BackedUp
            } else if current.as_deref() == original {
                Status::Intent
            } else {
                Status::Failed
            };
            if observed != entry.status {
                entry.status = observed;
                changed = true;
            }
        }
        if changed {
            save(root, &batch)?;
        }
        lines.push(format!("batch {}", batch.id));
        for entry in &batch.entries {
            lines.push(format!(
                "  {:?} {} backup {}",
                entry.status,
                entry.pending.expected.path.display(),
                entry.backup.display()
            ));
        }
    }
    Ok(lines)
}

pub fn undo(root: &Path, id: &str) -> Result<()> {
    let mut batch = load(root, id)?;
    for index in (0..batch.entries.len()).rev() {
        let entry = &batch.entries[index];
        validate_entry(root, entry)?;
        if entry.status != Status::Verified {
            if entry.status == Status::Failed && entry.written_hash.is_some() {
                let current = tags::hash_file(&entry.pending.expected.path)?;
                if current != entry.pending.expected.sha256.as_deref().unwrap_or_default() {
                    bail!(
                        "refusing undo: {} changed since apply",
                        entry.pending.expected.path.display()
                    );
                }
            }
            continue;
        }
        let path = &entry.pending.expected.path;
        if tags::hash_file(path)?
            != entry
                .written_hash
                .as_deref()
                .context("missing written hash")?
        {
            bail!("refusing undo: {} changed since apply", path.display());
        }
        if tags::hash_file(&entry.backup)?
            != entry
                .backup_hash
                .as_deref()
                .context("missing backup hash")?
        {
            bail!("backup changed: {}", entry.backup.display());
        }
        let temp = new_temp(path)?;
        fs::copy(&entry.backup, temp.path())?;
        temp.as_file().sync_all()?;
        temp.persist(path).map_err(|e| e.error)?;
        sync_parent(path)?;
        batch.entries[index].status = Status::Restored;
        save(root, &batch)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{FieldValue, NumberPair};

    #[test]
    fn old_staging_and_journal_migrate_without_losing_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("staging.json");
        let old = serde_json::json!([{
            "expected": {"path": "/music/song.mp3", "size": 7, "modified_ns": 8, "sha256": "abc"},
            "edits": [{"field": "track", "value": "3/12"}, {"field": "title", "value": "New"}],
            "before": [{"field": "title", "value": "Old"}]
        }]);
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let migrated = read_staging(&path).unwrap();
        assert_eq!(
            migrated[0].edits[0],
            EditOperation::Set {
                field: Field::Track,
                value: FieldValue::Number(NumberPair {
                    number: 3,
                    total: Some(12)
                }),
            }
        );
        assert_eq!(migrated[0].before[0].value.as_deref(), Some("Old"));

        let journal = dir.path().join("batch-1.json");
        let old_batch = serde_json::json!({
            "id": "1", "entries": [{
                "pending": old[0], "backup": "/tmp/backup.mp3", "backup_hash": null,
                "written_hash": null, "status": "Intent", "error": null
            }]
        });
        fs::write(&journal, serde_json::to_vec(&old_batch).unwrap()).unwrap();
        let batch = read_batch(&journal).unwrap();
        assert_eq!(batch.version, FORMAT_VERSION);
        assert_eq!(batch.entries[0].pending.edits, migrated[0].edits);
        durable_json(&journal, &batch).unwrap();
        assert_eq!(
            read_batch(&journal).unwrap().entries[0].pending.edits,
            migrated[0].edits
        );
    }

    #[test]
    fn unknown_persistence_versions_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("staging.json");
        fs::write(&path, br#"{"version":99,"entries":[]}"#).unwrap();
        assert!(
            read_staging(&path)
                .unwrap_err()
                .to_string()
                .contains("version 99")
        );
        let journal = dir.path().join("batch-1.json");
        fs::write(&journal, br#"{"version":99,"id":"1","entries":[]}"#).unwrap();
        assert!(
            read_batch(&journal)
                .unwrap_err()
                .to_string()
                .contains("version 99")
        );
    }
}
