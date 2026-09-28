//! Read-only projection of operation journals. Undo rechecks the live files.

use crate::{changes, operations, quarantine, rename};
use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    Edit,
    Rename,
    Quarantine,
}

impl OperationKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Edit => "Edited",
            Self::Rename => "Renamed",
            Self::Quarantine => "Quarantined",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationStatus {
    Completed,
    InProgress,
    Failed,
    Undone,
    Conflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verification {
    Verified,
    Changed,
    Missing,
    Unchecked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryKey {
    pub kind: OperationKind,
    pub id: String,
}

#[derive(Clone, Debug)]
pub struct HistoryFile {
    pub path: PathBuf,
    pub before: String,
    pub after: String,
    pub verification: Verification,
}

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub key: HistoryKey,
    pub timestamp_ns: u128,
    pub files: Vec<HistoryFile>,
    pub status: OperationStatus,
    pub verification: Verification,
    pub reversible: bool,
    pub undo_safe: bool,
}

impl HistoryEntry {
    pub fn summary(&self) -> String {
        let count = self.files.len();
        let noun = if count == 1 { "file" } else { "files" };
        format!("{} {count} {noun}", self.key.kind.label())
    }
    pub fn time_utc(&self) -> String {
        let minutes = (self.timestamp_ns / 1_000_000_000 / 60) % 1440;
        format!("{:02}:{:02} UTC", minutes / 60, minutes % 60)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("operation is no longer safe to undo")]
    UnsafeUndo,
    #[error("operation no longer exists")]
    NotFound,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

fn timestamp(id: &str) -> u128 {
    id.split('-')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn state(path: &Path, expected: Option<&str>) -> Verification {
    let Some(expected) = expected else {
        return Verification::Unchecked;
    };
    if !path.exists() {
        return Verification::Missing;
    }
    if path
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Verification::Changed;
    }
    match operations::fingerprint::matches_hash(path, expected) {
        Ok(true) => Verification::Verified,
        _ => Verification::Changed,
    }
}

fn aggregate(states: impl IntoIterator<Item = Verification>) -> Verification {
    let mut result = Verification::Verified;
    for state in states {
        result = match (result, state) {
            (Verification::Changed, _) | (_, Verification::Changed) => Verification::Changed,
            (Verification::Missing, _) | (_, Verification::Missing) => Verification::Missing,
            (Verification::Unchecked, _) | (_, Verification::Unchecked) => Verification::Unchecked,
            _ => Verification::Verified,
        };
    }
    result
}

fn edit_entry(root: &Path, batch: changes::Batch) -> HistoryEntry {
    let statuses: Vec<_> = batch
        .entries
        .iter()
        .map(|entry| entry.status.clone())
        .collect();
    let status = if !statuses.is_empty() && statuses.iter().all(|s| *s == changes::Status::Restored)
    {
        OperationStatus::Undone
    } else if statuses.contains(&changes::Status::Failed) {
        OperationStatus::Failed
    } else if !statuses.is_empty() && statuses.iter().all(|s| *s == changes::Status::Verified) {
        OperationStatus::Completed
    } else {
        OperationStatus::InProgress
    };
    let mut files = Vec::new();
    let mut undo_safe = status == OperationStatus::Completed;
    for entry in &batch.entries {
        let path = &entry.pending.expected.path;
        let valid = changes::validate_entry(root, entry).is_ok();
        let expected_current = if status == OperationStatus::Undone {
            entry.pending.expected.sha256.as_deref()
        } else {
            entry.written_hash.as_deref()
        };
        let current = if valid {
            state(path, expected_current)
        } else {
            Verification::Changed
        };
        let backup = if valid {
            state(&entry.backup, entry.backup_hash.as_deref())
        } else {
            Verification::Changed
        };
        let safe = valid
            && entry.backup_hash.as_deref() == entry.pending.expected.sha256.as_deref()
            && current == Verification::Verified
            && backup == Verification::Verified;
        undo_safe &= safe;
        let before = entry
            .pending
            .before
            .iter()
            .map(|old| {
                format!(
                    "{}={}",
                    old.field.label(),
                    old.value.as_deref().unwrap_or("∅")
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let after = entry
            .pending
            .edits
            .iter()
            .map(|edit| format!("{}={}", edit.field().label(), changes::describe_edit(edit)))
            .collect::<Vec<_>>()
            .join(", ");
        files.push(HistoryFile {
            path: path.clone(),
            before,
            after,
            verification: aggregate([current, backup]),
        });
    }
    let verification = aggregate(files.iter().map(|file| file.verification));
    HistoryEntry {
        key: HistoryKey {
            kind: OperationKind::Edit,
            id: batch.id.clone(),
        },
        timestamp_ns: timestamp(&batch.id),
        files,
        status,
        verification,
        reversible: true,
        undo_safe,
    }
}

fn rename_entry(root: &Path, journal: rename::Journal) -> HistoryEntry {
    let phases: Vec<_> = journal.moves.iter().map(|item| item.phase).collect();
    let status =
        if !phases.is_empty() && phases.iter().all(|phase| *phase == rename::Phase::Restored) {
            OperationStatus::Undone
        } else if !phases.is_empty() && phases.iter().all(|phase| *phase == rename::Phase::Moved) {
            OperationStatus::Completed
        } else {
            OperationStatus::InProgress
        };
    let mut files = Vec::new();
    let mut undo_safe = status == OperationStatus::Completed;
    for item in &journal.moves {
        let operation = &item.operation;
        let valid = operation.source.starts_with(root)
            && operation.destination.starts_with(root)
            && operations::preflight::symlink_component(root, &operation.source).ok() == Some(None)
            && operations::preflight::symlink_component(root, &operation.destination).ok()
                == Some(None)
            && operation
                .destination
                .canonicalize()
                .map_or(status == OperationStatus::Undone, |path| {
                    path.starts_with(root)
                });
        let current = if !valid {
            Verification::Changed
        } else if status == OperationStatus::Undone {
            if operation.destination.exists() {
                Verification::Changed
            } else {
                state(&operation.source, Some(&operation.sha256))
            }
        } else if operation.source.exists() {
            Verification::Changed
        } else {
            state(&operation.destination, Some(&operation.sha256))
        };
        let safe = valid && !operation.source.exists() && current == Verification::Verified;
        undo_safe &= safe;
        files.push(HistoryFile {
            path: operation.destination.clone(),
            before: operation.source.display().to_string(),
            after: operation.destination.display().to_string(),
            verification: current,
        });
    }
    let verification = aggregate(files.iter().map(|file| file.verification));
    HistoryEntry {
        key: HistoryKey {
            kind: OperationKind::Rename,
            id: journal.id.clone(),
        },
        timestamp_ns: timestamp(&journal.id),
        files,
        status,
        verification,
        reversible: true,
        undo_safe,
    }
}

fn quarantine_entry(record: quarantine::Record) -> HistoryEntry {
    let status = match record.status {
        quarantine::Status::Moved => OperationStatus::Completed,
        quarantine::Status::Restored => OperationStatus::Undone,
        quarantine::Status::Conflict => OperationStatus::Conflict,
        _ => OperationStatus::InProgress,
    };
    let verification = if status == OperationStatus::Conflict {
        Verification::Changed
    } else if status == OperationStatus::Undone {
        if record.quarantined.exists() {
            Verification::Changed
        } else {
            state(&record.original, Some(&record.sha256))
        }
    } else if record.original.exists() {
        Verification::Changed
    } else {
        state(&record.quarantined, Some(&record.sha256))
    };
    let undo_safe = status == OperationStatus::Completed
        && verification == Verification::Verified
        && !record.original.exists();
    HistoryEntry {
        key: HistoryKey {
            kind: OperationKind::Quarantine,
            id: record.id.clone(),
        },
        timestamp_ns: timestamp(&record.id),
        files: vec![HistoryFile {
            path: record.quarantined.clone(),
            before: record.original.display().to_string(),
            after: record.quarantined.display().to_string(),
            verification,
        }],
        status,
        verification,
        reversible: true,
        undo_safe,
    }
}

pub fn list(root: &Path) -> Result<Vec<HistoryEntry>, HistoryError> {
    let mut entries = Vec::new();
    for batch in changes::history_batches(root)? {
        entries.push(edit_entry(root, batch));
    }
    for journal in rename::history_journals(root)? {
        entries.push(rename_entry(root, journal));
    }
    for record in quarantine::history_records(root)? {
        entries.push(quarantine_entry(record));
    }
    entries.sort_by(|a, b| {
        b.timestamp_ns
            .cmp(&a.timestamp_ns)
            .then_with(|| b.key.id.cmp(&a.key.id))
    });
    Ok(entries)
}

pub fn undo(root: &Path, key: &HistoryKey) -> Result<(), HistoryError> {
    let current = list(root)?
        .into_iter()
        .find(|entry| &entry.key == key)
        .ok_or(HistoryError::NotFound)?;
    if !current.undo_safe {
        return Err(HistoryError::UnsafeUndo);
    }
    match key.kind {
        OperationKind::Edit => {
            changes::undo(root, &key.id).map_err(|error| HistoryError::Other(error.into()))?
        }
        OperationKind::Rename => rename::undo(root, &key.id)?,
        OperationKind::Quarantine => {
            quarantine::restore(root, &quarantine::journal_dir(root)?, &key.id)?
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renamed_file_modified_externally_cannot_be_undone() {
        let root = tempfile::tempdir().expect("temporary library");
        let source = root.path().join("old.mp3");
        let destination = root.path().join("new.mp3");
        std::fs::write(&destination, b"original").expect("destination");
        let hash = operations::fingerprint::hash_file(&destination).expect("hash");
        let make_entry = || {
            rename_entry(
                root.path(),
                rename::Journal {
                    id: "1700000000000000000-1".into(),
                    moves: vec![rename::JournalMove {
                        operation: rename::Move {
                            source: source.clone(),
                            destination: destination.clone(),
                            sha256: hash.clone(),
                        },
                        phase: rename::Phase::Moved,
                    }],
                },
            )
        };
        let initial = make_entry();
        assert_eq!(initial.status, OperationStatus::Completed);
        assert_eq!(initial.verification, Verification::Verified);
        assert!(initial.undo_safe);

        std::fs::write(&destination, b"external update").expect("external update");
        let changed = make_entry();
        assert_eq!(changed.verification, Verification::Changed);
        assert!(!changed.undo_safe);
    }

    #[test]
    fn rename_undo_requires_vacant_original_path() {
        let root = tempfile::tempdir().expect("temporary library");
        let source = root.path().join("old.mp3");
        let destination = root.path().join("new.mp3");
        std::fs::write(&source, b"different file").expect("occupied source");
        std::fs::write(&destination, b"original").expect("destination");
        let hash = operations::fingerprint::hash_file(&destination).expect("hash");
        let entry = rename_entry(
            root.path(),
            rename::Journal {
                id: "1700000000000000000-1".into(),
                moves: vec![rename::JournalMove {
                    operation: rename::Move {
                        source,
                        destination,
                        sha256: hash,
                    },
                    phase: rename::Phase::Moved,
                }],
            },
        );
        assert!(!entry.undo_safe);
        assert_eq!(entry.verification, Verification::Changed);
    }

    #[test]
    fn quarantined_file_modified_externally_cannot_be_restored() {
        let root = tempfile::tempdir().expect("temporary library");
        let quarantined = root.path().join("quarantined.mp3");
        std::fs::write(&quarantined, b"original").expect("quarantined file");
        let hash = operations::fingerprint::hash_file(&quarantined).expect("hash");
        let make_entry = || {
            quarantine_entry(quarantine::Record {
                id: "1700000000000000000-1".into(),
                original: root.path().join("original.mp3"),
                quarantined: quarantined.clone(),
                peer: root.path().join("peer.mp3"),
                sha256: hash.clone(),
                status: quarantine::Status::Moved,
            })
        };
        assert!(make_entry().undo_safe);
        std::fs::write(&quarantined, b"external update").expect("external update");
        let changed = make_entry();
        assert_eq!(changed.verification, Verification::Changed);
        assert!(!changed.undo_safe);
    }
}
