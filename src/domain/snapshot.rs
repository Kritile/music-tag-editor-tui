use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    pub path: PathBuf,
    pub size: u64,
    pub modified_ns: u128,
    pub sha256: Option<String>,
}
