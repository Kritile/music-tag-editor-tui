use serde::{Deserialize, Serialize};

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
