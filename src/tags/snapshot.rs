use crate::domain::{AudioFormat, Snapshot};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::path::Path;
use std::time::UNIX_EPOCH;

pub fn format(path: &Path) -> Option<AudioFormat> {
    AudioFormat::from_extension(path.extension()?.to_str()?)
}

pub fn snapshot(path: &Path, fingerprint: bool) -> Result<Snapshot> {
    let stat = fs::metadata(path).with_context(|| format!("metadata: {}", path.display()))?;
    let modified_ns = stat.modified()?.duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(Snapshot {
        path: path.to_path_buf(),
        size: stat.len(),
        modified_ns,
        sha256: fingerprint.then(|| hash_file(path)).transpose()?,
    })
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}
