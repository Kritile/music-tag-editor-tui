use super::{AudioFormat, Metadata, RawTag, Snapshot, TrackId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub snapshot: Snapshot,
    pub format: AudioFormat,
    pub raw: Vec<RawTag>,
    pub metadata: Metadata,
    pub diagnostics: Vec<String>,
    pub writable: bool,
    pub write_reason: String,
}
