use crate::domain::Snapshot;
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::path::Path;

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) fn matches_hash(path: &Path, expected: &str) -> Result<bool> {
    Ok(hash_file(path)? == expected)
}

pub(crate) fn matches_snapshot(expected: &Snapshot) -> Result<bool> {
    Ok(expected.sha256.is_some() && crate::tags::snapshot(&expected.path, true)? == *expected)
}
