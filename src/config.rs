use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

const VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub last_root: Option<PathBuf>,
    #[serde(default)]
    pub library: LibraryConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LibraryConfig {
    pub watch: bool,
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self { watch: true }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: VERSION,
            last_root: None,
            library: LibraryConfig::default(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigV1 {
    version: u32,
    last_root: Option<PathBuf>,
}

fn config_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("org", "music-tag-editor", "music-tag-editor")
        .context("cannot determine application configuration directory")?;
    Ok(dirs.config_dir().join("config.toml"))
}

fn parse(text: &str) -> Result<Config> {
    let value: toml::Value = toml::from_str(text)?;
    let version = value
        .get("version")
        .and_then(toml::Value::as_integer)
        .context("configuration version is missing or invalid")?;
    match version {
        1 => {
            let old: ConfigV1 = value.try_into()?;
            let _ = old.version;
            Ok(Config {
                last_root: old.last_root,
                ..Config::default()
            })
        }
        2 => Ok(value.try_into()?),
        _ => bail!("unsupported configuration version {version}"),
    }
}

fn load_from(path: &Path) -> Result<Config> {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(error.into()),
    }
}

/// Load the current configuration, migrating supported older versions in memory.
pub fn load() -> Result<Config> {
    load_from(&config_path()?)
}

fn save_to(path: &Path, config: &Config) -> Result<()> {
    let parent = path.parent().context("missing configuration parent")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(toml::to_string_pretty(config)?.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn remember(root: &Path) -> Result<()> {
    let path = config_path()?;
    let mut config = load_from(&path)?;
    config.last_root = Some(root.to_path_buf());
    save_to(&path, &config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_migrates_root_and_adds_watcher_default() {
        let old = "version = 1\nlast_root = '/music'\n";
        assert_eq!(
            parse(old).expect("v1 config"),
            Config {
                last_root: Some(PathBuf::from("/music")),
                ..Config::default()
            }
        );
    }

    #[test]
    fn v2_roundtrip_preserves_watcher_preference_when_root_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let mut config = parse("version = 2\n[library]\nwatch = false\n").expect("v2 config");
        config.last_root = Some(PathBuf::from("/new-music"));
        save_to(&path, &config).expect("save");
        assert_eq!(load_from(&path).expect("reload"), config);
        assert!(!load_from(&path).expect("reload").library.watch);
    }

    #[test]
    fn future_and_unknown_fields_are_rejected_without_rewrite() {
        assert!(parse("version = 3\n").is_err());
        assert!(parse("version = 2\nunknown = true\n").is_err());
    }
}
