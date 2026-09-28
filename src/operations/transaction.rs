use crate::operations::fingerprint;
use anyhow::{Context, Result};
use std::fs::{self, File};
use std::path::Path;
use tempfile::NamedTempFile;

pub(crate) fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(crate) fn sync_parent(path: &Path) -> Result<()> {
    sync_dir(path.parent().context("missing parent directory")?)
}

pub(crate) fn replace(temp: NamedTempFile, target: &Path) -> Result<()> {
    temp.as_file().sync_all()?;
    temp.persist(target).map_err(|error| error.error)?;
    sync_parent(target)
}

pub(crate) fn create_noclobber(temp: NamedTempFile, target: &Path) -> Result<()> {
    temp.as_file().sync_all()?;
    temp.persist_noclobber(target)
        .map_err(|error| error.error)?;
    sync_parent(target)
}

pub(crate) fn rename_and_sync(source: &Path, target: &Path) -> Result<()> {
    fs::rename(source, target)?;
    sync_parent(source)?;
    if source.parent() != target.parent() {
        sync_parent(target)?;
    }
    Ok(())
}

/// Copies and syncs bytes, then checks the fingerprint. `target` must be an
/// operation-owned temporary or unique backup path because `fs::copy` overwrites it.
pub(crate) fn copy_verified(source: &Path, target: &Path, expected: &str) -> Result<bool> {
    fs::copy(source, target)?;
    File::open(target)?.sync_all()?;
    fingerprint::matches_hash(target, expected)
}
