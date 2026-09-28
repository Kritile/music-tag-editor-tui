use super::Field;
use serde::{Deserialize, Serialize};

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

impl Metadata {
    pub fn value(&self, field: Field) -> Option<String> {
        match field {
            Field::Title => self.title.clone(),
            Field::Artist => self.artist.clone(),
            Field::AlbumArtist => self.album_artist.clone(),
            Field::Album => self.album.clone(),
            Field::Track => self.track.as_ref().map(format_number),
            Field::Disc => self.disc.as_ref().map(format_number),
            Field::Date => self.date.clone(),
            Field::Artists => (!self.artists.is_empty()).then(|| self.artists.join("; ")),
            Field::Genres => (!self.genres.is_empty()).then(|| self.genres.join("; ")),
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
