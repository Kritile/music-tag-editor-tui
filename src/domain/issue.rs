use super::{Edit, TrackId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Observed,
    Likely,
    Experimental,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub rule_id: String,
    pub track_id: TrackId,
    pub path: PathBuf,
    pub description: String,
    pub confidence: Confidence,
    pub verification: String,
    pub suggestion: Option<Edit>,
}
