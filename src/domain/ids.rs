use serde::{Deserialize, Deserializer, Serialize, de::Error};
use std::fmt;

/// SQLite identity of an indexed track. Zero is reserved for a track not yet indexed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct TrackId(i64);

impl TrackId {
    pub const UNINDEXED: Self = Self(0);

    pub const fn indexed(value: i64) -> Option<Self> {
        if value > 0 { Some(Self(value)) } else { None }
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for TrackId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = i64::deserialize(deserializer)?;
        if value < 0 {
            return Err(D::Error::custom("track ID cannot be negative"));
        }
        Ok(Self(value))
    }
}

impl fmt::Display for TrackId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
