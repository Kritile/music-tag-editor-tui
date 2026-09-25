use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    pub path: PathBuf,
    pub size: u64,
    pub modified_ns: u128,
    pub sha256: Option<String>,
}

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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NumberPair {
    pub number: u32,
    pub total: Option<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Metadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub artists: Vec<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub track: Option<NumberPair>,
    pub disc: Option<NumberPair>,
    pub date: Option<String>,
    pub genres: Vec<String>,
    pub artwork_count: usize,
    pub release_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub id: i64,
    pub snapshot: Snapshot,
    pub format: String,
    pub raw: Vec<RawTag>,
    pub metadata: Metadata,
    pub diagnostics: Vec<String>,
    pub writable: bool,
    pub write_reason: String,
}

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
    pub track_id: i64,
    pub path: PathBuf,
    pub description: String,
    pub confidence: Confidence,
    pub verification: String,
    pub suggestion: Option<Edit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Edit {
    pub field: Field,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Title,
    Artist,
    AlbumArtist,
    Album,
    Track,
    Disc,
}

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "TITLE",
            Self::Artist => "ARTIST",
            Self::AlbumArtist => "ALBUMARTIST",
            Self::Album => "ALBUM",
            Self::Track => "TRACKNUMBER",
            Self::Disc => "DISCNUMBER",
        }
    }
}

impl Metadata {
    pub fn value(&self, field: Field) -> Option<String> {
        match field {
            Field::Title => self.title.clone(),
            Field::Artist => self.artist.clone(),
            Field::AlbumArtist => self.album_artist.clone(),
            Field::Album => self.album.clone(),
            Field::Track => self.track.as_ref().map(format_number),
            Field::Disc => self.disc.as_ref().map(format_number),
        }
    }
}

fn format_number(pair: &NumberPair) -> String {
    match pair.total {
        Some(total) => format!("{}/{total}", pair.number),
        None => pair.number.to_string(),
    }
}

pub fn parse_number(value: &str) -> Option<NumberPair> {
    let (number, total) = match value.trim().split_once('/') {
        Some((number, total)) => (
            number.trim().parse().ok()?,
            Some(total.trim().parse().ok()?),
        ),
        None => (value.trim().parse().ok()?, None),
    };
    if number == 0 || total == Some(0) || total.is_some_and(|n| number > n) {
        return None;
    }
    Some(NumberPair { number, total })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_pairs_reject_invalid_values() {
        assert_eq!(
            parse_number("3/12"),
            Some(NumberPair {
                number: 3,
                total: Some(12)
            })
        );
        assert_eq!(parse_number("0/12"), None);
        assert_eq!(parse_number("13/12"), None);
        assert_eq!(parse_number("3/no"), None);
    }
}
