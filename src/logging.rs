use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::sync::Mutex;
use tracing_subscriber::EnvFilter;

/// Enable file logging only when RUST_LOG is set, keeping the active TUI clean.
pub(crate) fn init() -> Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        return Ok(());
    }
    let filter = EnvFilter::try_from_default_env().context("invalid RUST_LOG filter")?;
    let path = crate::library::data_dir()?.join("music-tui.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("cannot open diagnostic log {}", path.display()))?;
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(Mutex::new(file))
        .try_init()
        .map_err(|error| anyhow::anyhow!("cannot install diagnostic subscriber: {error}"))?;
    Ok(())
}
