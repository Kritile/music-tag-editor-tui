use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RawValue {
    Text(String),
    Locator(String),
    Binary { bytes: usize, sha256: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawTag {
    pub container: String,
    pub native_key: String,
    pub key: String,
    pub value: RawValue,
    pub order: usize,
    #[serde(default)]
    pub detail: String,
}
