use anyhow::{Context, Result};
use serde::Serialize;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

const VERSION: u32 = 1;

#[derive(Serialize)]
pub struct Config {
    version: u32,
    last_root: Option<PathBuf>,
}

fn config_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("org", "music-tag-editor", "music-tag-editor")
        .context("cannot determine application configuration directory")?;
    Ok(dirs.config_dir().join("config.toml"))
}

pub fn remember(root: &Path) -> Result<()> {
    let path = config_path()?;
    let parent = path.parent().context("missing configuration parent")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(
        toml::to_string_pretty(&Config {
            version: VERSION,
            last_root: Some(root.to_path_buf()),
        })?
        .as_bytes(),
    )?;
    temp.as_file().sync_all()?;
    temp.persist(&path).map_err(|e| e.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
