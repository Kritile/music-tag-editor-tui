use serde::{Deserialize, Serialize};
use std::fmt;

/// Supported audio containers. Serialized names match existing index and report data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioFormat {
    #[serde(rename = "MP3")]
    Mp3,
    #[serde(rename = "FLAC")]
    Flac,
    #[serde(rename = "MP4")]
    Mp4,
    #[serde(rename = "Ogg")]
    OggVorbis,
    #[serde(rename = "Opus")]
    Opus,
}

impl AudioFormat {
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "mp3" => Some(Self::Mp3),
            "flac" => Some(Self::Flac),
            "m4a" | "mp4" => Some(Self::Mp4),
            "ogg" | "oga" => Some(Self::OggVorbis),
            "opus" => Some(Self::Opus),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "MP3",
            Self::Flac => "FLAC",
            Self::Mp4 => "MP4",
            Self::OggVorbis => "Ogg",
            Self::Opus => "Opus",
        }
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
