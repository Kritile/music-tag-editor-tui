use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Returns the first symlink below `root`, including the target if it exists.
pub(crate) fn symlink_component(root: &Path, path: &Path) -> Result<Option<PathBuf>> {
    let mut current = root.to_path_buf();
    for component in path.strip_prefix(root)?.components() {
        current.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&current)
            && metadata.file_type().is_symlink()
        {
            return Ok(Some(current));
        }
    }
    Ok(None)
}

pub(crate) fn has_space(path: &Path, required: u64) -> Result<bool> {
    Ok(fs2::available_space(path)? >= required)
}
