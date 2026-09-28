use super::snapshot::{format, snapshot};
use crate::domain::{Metadata, RawTag, RawValue, Track, TrackId, parse_number};
use anyhow::{Context, Result};
use lofty::file::TaggedFileExt;
use lofty::probe::read_from_path;
use lofty::tag::{ItemKey, ItemValue};
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn read_track(path: &Path) -> Result<Track> {
    let fmt = format(path).context("unsupported audio extension")?;
    let file = read_from_path(path).with_context(|| format!("read tags: {}", path.display()))?;
    let mut raw = Vec::new();
    let mut diagnostics = Vec::new();
    let mut metadata = Metadata::default();
    let primary = file.primary_tag().or_else(|| file.first_tag());
    for tag in file.tags() {
        let container = format!("{:?}", tag.tag_type());
        for (order, item) in tag.items().enumerate() {
            let key = item.key();
            let value = match item.value() {
                ItemValue::Text(s) => RawValue::Text(s.clone()),
                ItemValue::Locator(s) => RawValue::Locator(s.clone()),
                ItemValue::Binary(data) => RawValue::Binary {
                    bytes: data.len(),
                    sha256: format!("{:x}", Sha256::digest(data)),
                },
            };
            raw.push(RawTag {
                container: container.clone(),
                native_key: key
                    .map_key(tag.tag_type(), true)
                    .unwrap_or("<unmapped>")
                    .to_string(),
                key: format!("{key:?}"),
                value,
                order,
                detail: format!(
                    "description={:?}; language={:?}",
                    item.description(),
                    item.lang()
                ),
            });
        }
        for (order, picture) in tag.pictures().iter().enumerate() {
            raw.push(RawTag {
                container: container.clone(),
                native_key: format!("PICTURE[{order}]"),
                key: "Artwork".to_string(),
                value: RawValue::Binary {
                    bytes: picture.data().len(),
                    sha256: format!("{:x}", Sha256::digest(picture.data())),
                },
                order: tag.item_count() as usize + order,
                detail: format!(
                    "type={:?}; mime={:?}; description={:?}",
                    picture.pic_type(),
                    picture.mime_type(),
                    picture.description()
                ),
            });
        }
    }
    if let Some(tag) = primary {
        let get = |key: ItemKey| tag.get_string(&key).map(str::to_owned);
        metadata.title = get(ItemKey::TrackTitle);
        metadata.artist = get(ItemKey::TrackArtist);
        metadata.album_artist = get(ItemKey::AlbumArtist);
        metadata.album = get(ItemKey::AlbumTitle);
        metadata.date = get(ItemKey::RecordingDate);
        metadata.release_id = get(ItemKey::MusicBrainzReleaseId);
        metadata.artwork_count = tag.picture_count() as usize;
        metadata.artists = tag
            .get_strings(&ItemKey::TrackArtists)
            .map(str::to_owned)
            .collect();
        metadata.genres = tag
            .get_strings(&ItemKey::Genre)
            .map(str::to_owned)
            .collect();
        let number = get(ItemKey::TrackNumber);
        let total = get(ItemKey::TrackTotal);
        metadata.track = number.as_deref().and_then(parse_number);
        if let (Some(pair), Some(total)) = (&mut metadata.track, total) {
            pair.total = total.parse().ok().or(pair.total);
        }
        let number = get(ItemKey::DiscNumber);
        let total = get(ItemKey::DiscTotal);
        metadata.disc = number.as_deref().and_then(parse_number);
        if let (Some(pair), Some(total)) = (&mut metadata.disc, total) {
            pair.total = total.parse().ok().or(pair.total);
        }
        if let Some(value) = number.as_deref()
            && metadata.disc.is_none()
        {
            diagnostics.push(format!("Invalid disc number: {value}"));
        }
    }
    for field in [
        ItemKey::TrackArtist,
        ItemKey::AlbumArtist,
        ItemKey::AlbumTitle,
    ] {
        let mut values = Vec::new();
        for tag in file.tags() {
            for text in tag.get_strings(&field) {
                if !values.contains(&text) {
                    values.push(text);
                }
            }
        }
        if values.len() > 1 {
            diagnostics.push(format!(
                "Conflicting {field:?} across tag containers: {}",
                values.join(" | ")
            ));
        }
    }
    let capabilities = super::adapters::for_format(fmt).capabilities(&file);
    Ok(Track {
        id: TrackId::UNINDEXED,
        snapshot: snapshot(path, false)?,
        format: fmt,
        raw,
        metadata,
        diagnostics,
        writable: capabilities.writable,
        write_reason: capabilities.reason,
    })
}
