use crate::domain::{Edit, Field, Track, TrackId};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTrack {
    pub disc: u32,
    pub number: u32,
    pub title: String,
    pub artist: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub date: Option<String>,
    pub country: Option<String>,
    pub tracks: u32,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub title: String,
    pub artist: String,
    pub tracks: Vec<RemoteTrack>,
}

#[derive(Clone, Debug)]
pub struct Proposal {
    pub local_id: TrackId,
    pub edit: Edit,
    pub before: Option<String>,
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn match_tracks(local: &[Track], remote: &[RemoteTrack]) -> Vec<Option<usize>> {
    let mut used = vec![false; remote.len()];
    local
        .iter()
        .map(|track| {
            let number = track.metadata.track.as_ref().map(|n| n.number);
            let disc = track.metadata.disc.as_ref().map_or(1, |n| n.number);
            let title = track.metadata.title.as_deref().map(normalize);
            let numbered: Vec<_> = remote
                .iter()
                .enumerate()
                .filter(|(index, item)| {
                    !used[*index] && Some(item.number) == number && item.disc == disc
                })
                .map(|(index, _)| index)
                .collect();
            let candidates: Vec<_> = if numbered.len() == 1
                && title
                    .as_ref()
                    .is_some_and(|title| normalize(&remote[numbered[0]].title) == *title)
            {
                numbered
            } else if title.is_some() {
                remote
                    .iter()
                    .enumerate()
                    .filter(|(index, item)| {
                        !used[*index]
                            && title
                                .as_ref()
                                .is_some_and(|title| normalize(&item.title) == *title)
                    })
                    .map(|(index, _)| index)
                    .collect()
            } else {
                Vec::new()
            };
            if candidates.len() == 1 {
                used[candidates[0]] = true;
                Some(candidates[0])
            } else {
                None
            }
        })
        .collect()
}

pub fn proposals(track: &Track, remote: &RemoteTrack, release: &Release) -> Vec<Proposal> {
    let number = |current: Option<&crate::domain::NumberPair>, proposed: u32| {
        if current.is_some_and(|pair| pair.number == proposed) {
            None
        } else {
            Some(
                match current
                    .and_then(|pair| pair.total)
                    .filter(|total| proposed <= *total)
                {
                    Some(total) => format!("{proposed}/{total}"),
                    None => proposed.to_string(),
                },
            )
        }
    };
    let values = [
        (Field::Title, Some(remote.title.clone())),
        (Field::Artist, Some(remote.artist.clone())),
        (Field::AlbumArtist, Some(release.artist.clone())),
        (Field::Album, Some(release.title.clone())),
        (
            Field::Track,
            number(track.metadata.track.as_ref(), remote.number),
        ),
        (
            Field::Disc,
            number(track.metadata.disc.as_ref(), remote.disc),
        ),
    ];
    values
        .into_iter()
        .filter_map(|(field, value)| {
            let value = value?;
            if value.trim().is_empty() {
                return None;
            }
            let before = track.metadata.value(field);
            (before.as_deref() != Some(value.as_str())).then_some(Proposal {
                local_id: track.id,
                edit: Edit { field, value },
                before,
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct SearchResponse {
    releases: Vec<SearchRelease>,
}

#[derive(Deserialize)]
struct SearchRelease {
    id: String,
    title: String,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default, rename = "track-count")]
    track_count: u32,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ArtistCredit {
    Text(String),
    Name { name: String },
}

fn credit(parts: &[ArtistCredit]) -> String {
    parts
        .iter()
        .map(|part| match part {
            ArtistCredit::Text(value) => value.as_str(),
            ArtistCredit::Name { name } => name.as_str(),
        })
        .collect()
}

#[derive(Deserialize)]
struct ReleaseResponse {
    title: String,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
    #[serde(default)]
    media: Vec<Medium>,
}

#[derive(Deserialize)]
struct Medium {
    position: u32,
    #[serde(default)]
    tracks: Vec<ReleaseTrack>,
}

#[derive(Deserialize)]
struct ReleaseTrack {
    position: u32,
    title: String,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
    #[serde(default)]
    recording: Option<Recording>,
}

#[derive(Deserialize)]
struct Recording {
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
}

fn parse_release(value: ReleaseResponse) -> Release {
    let artist = credit(&value.artist_credit);
    let tracks = value
        .media
        .into_iter()
        .flat_map(|medium| {
            let artist = artist.clone();
            medium.tracks.into_iter().map(move |track| {
                let track_artist = credit(&track.artist_credit);
                let recording_artist = track
                    .recording
                    .as_ref()
                    .map(|item| credit(&item.artist_credit))
                    .unwrap_or_default();
                RemoteTrack {
                    disc: medium.position,
                    number: track.position,
                    title: track.title,
                    artist: if !track_artist.is_empty() {
                        track_artist
                    } else if !recording_artist.is_empty() {
                        recording_artist
                    } else {
                        artist.clone()
                    },
                }
            })
        })
        .collect();
    Release {
        title: value.title,
        artist,
        tracks,
    }
}

pub struct Client {
    http: reqwest::blocking::Client,
}

static LAST_REQUEST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

impl Client {
    pub fn new() -> Result<Self> {
        let agent = format!(
            "music-tui/{} (https://github.com/Kritile/music-tag-editor-tui)",
            env!("CARGO_PKG_VERSION")
        );
        let http = reqwest::blocking::Client::builder()
            .user_agent(agent)
            .timeout(Duration::from_secs(15))
            .https_only(true)
            .build()?;
        Ok(Self { http })
    }

    fn get<T: serde::de::DeserializeOwned>(
        &mut self,
        url: &str,
        params: &[(&str, &str)],
    ) -> Result<T> {
        let mut last_request = LAST_REQUEST
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| anyhow::anyhow!("MusicBrainz request clock poisoned"))?;
        if let Some(last) = *last_request {
            let elapsed = last.elapsed();
            if elapsed < Duration::from_secs(1) {
                std::thread::sleep(Duration::from_secs(1) - elapsed);
            }
        }
        *last_request = Some(Instant::now());
        drop(last_request);
        let response = self
            .http
            .get(url)
            .query(params)
            .send()?
            .error_for_status()?;
        Ok(response.json()?)
    }

    pub fn search(&mut self, artist: &str, album: &str) -> Result<Vec<Candidate>> {
        if artist.trim().is_empty() || album.trim().is_empty() {
            bail!("album and artist are required for MusicBrainz search");
        }
        let query = format!(
            "release:\"{}\" AND artist:\"{}\"",
            escape_query(album),
            escape_query(artist)
        );
        let response: SearchResponse = self
            .get(
                "https://musicbrainz.org/ws/2/release/",
                &[("query", &query), ("fmt", "json"), ("limit", "25")],
            )
            .context("MusicBrainz release search failed")?;
        Ok(response
            .releases
            .into_iter()
            .map(|release| Candidate {
                id: release.id,
                title: release.title,
                artist: credit(&release.artist_credit),
                date: release.date,
                country: release.country,
                tracks: release.track_count,
            })
            .collect())
    }

    pub fn release(&mut self, id: &str) -> Result<Release> {
        if !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') || id.len() != 36 {
            bail!("invalid MusicBrainz release ID");
        }
        let response: ReleaseResponse = self
            .get(
                &format!("https://musicbrainz.org/ws/2/release/{id}"),
                &[("inc", "recordings+artist-credits"), ("fmt", "json")],
            )
            .context("MusicBrainz release lookup failed")?;
        Ok(parse_release(response))
    }
}

fn escape_query(input: &str) -> String {
    input
        .chars()
        .flat_map(|ch| {
            if "\\+-!():^[]{}\"~*?|&/".contains(ch) {
                vec!['\\', ch]
            } else {
                vec![ch]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AudioFormat, Metadata, NumberPair, Snapshot};
    use std::path::PathBuf;

    fn local(id: i64, title: &str, number: u32) -> Track {
        Track {
            id: TrackId::indexed(id).expect("indexed fixture"),
            snapshot: Snapshot {
                path: PathBuf::from(format!("/music/{id}.mp3")),
                size: 0,
                modified_ns: 0,
                sha256: None,
            },
            format: AudioFormat::Mp3,
            raw: Vec::new(),
            metadata: Metadata {
                title: Some(title.into()),
                track: Some(NumberPair {
                    number,
                    total: None,
                }),
                disc: Some(NumberPair {
                    number: 1,
                    total: None,
                }),
                ..Metadata::default()
            },
            diagnostics: Vec::new(),
            writable: true,
            write_reason: String::new(),
        }
    }

    #[test]
    fn matches_unique_disc_and_number_but_leaves_conflicting_titles_unmatched() {
        let local = vec![local(1, "Opening", 1), local(2, "Elsewhere", 2)];
        let remote = vec![
            RemoteTrack {
                disc: 1,
                number: 1,
                title: "Opening".into(),
                artist: "Band".into(),
            },
            RemoteTrack {
                disc: 1,
                number: 2,
                title: "Different Song".into(),
                artist: "Band".into(),
            },
        ];
        assert_eq!(match_tracks(&local, &remote), vec![Some(0), None]);
    }

    #[test]
    fn release_json_preserves_disc_positions_and_track_credit() {
        let json = r#"{"id":"11111111-1111-1111-1111-111111111111","title":"Album","artist-credit":[{"name":"Band"}],"media":[{"position":2,"tracks":[{"position":3,"title":"Song","artist-credit":[{"name":"Guest"}]}]}]}"#;
        let response: ReleaseResponse = serde_json::from_str(json).expect("release JSON");
        let release = parse_release(response);
        assert_eq!(
            release.tracks,
            vec![RemoteTrack {
                disc: 2,
                number: 3,
                title: "Song".into(),
                artist: "Guest".into()
            }]
        );
    }

    #[test]
    fn proposals_only_include_changed_supported_fields() {
        let mut local = local(1, "Old", 1);
        local.metadata.album = Some("Album".into());
        let release = Release {
            title: "Album".into(),
            artist: "Band".into(),
            tracks: vec![],
        };
        let remote = RemoteTrack {
            disc: 1,
            number: 1,
            title: "New".into(),
            artist: "Band".into(),
        };
        let changes = proposals(&local, &remote, &release);
        assert!(
            changes
                .iter()
                .any(|p| p.edit.field == Field::Title && p.edit.value == "New")
        );
        assert!(!changes.iter().any(|p| p.edit.field == Field::Album));
    }

    #[test]
    fn matching_numbers_do_not_erase_local_totals() {
        let mut local = local(1, "Song", 1);
        local.metadata.track.as_mut().expect("track").total = Some(12);
        local.metadata.disc.as_mut().expect("disc").total = Some(2);
        let release = Release {
            title: "Album".into(),
            artist: "Band".into(),
            tracks: vec![],
        };
        let remote = RemoteTrack {
            disc: 1,
            number: 1,
            title: "Song".into(),
            artist: "Band".into(),
        };
        let changes = proposals(&local, &remote, &release);
        assert!(
            !changes
                .iter()
                .any(|p| matches!(p.edit.field, Field::Track | Field::Disc))
        );
    }

    #[test]
    fn missing_remote_credit_does_not_suggest_clearing_artist() {
        let mut local = local(1, "Song", 1);
        local.metadata.artist = Some("Band".into());
        local.metadata.album_artist = Some("Band".into());
        let release = Release {
            title: "Album".into(),
            artist: String::new(),
            tracks: vec![],
        };
        let remote = RemoteTrack {
            disc: 1,
            number: 1,
            title: "Song".into(),
            artist: String::new(),
        };
        let changes = proposals(&local, &remote, &release);
        assert!(
            !changes
                .iter()
                .any(|p| matches!(p.edit.field, Field::Artist | Field::AlbumArtist))
        );
    }
}
