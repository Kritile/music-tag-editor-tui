use crate::domain::{
    AudioFormat, EditOperation, Field, FieldValue, Metadata, RawTag, RawValue, Snapshot, Track,
    TrackId, parse_number,
};
use anyhow::{Context, Result, bail};
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::read_from_path;
use lofty::tag::{ItemKey, ItemValue, TagItem, TagType};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

pub fn format(path: &Path) -> Option<AudioFormat> {
    AudioFormat::from_extension(path.extension()?.to_str()?)
}

pub fn snapshot(path: &Path, fingerprint: bool) -> Result<Snapshot> {
    let stat = fs::metadata(path).with_context(|| format!("metadata: {}", path.display()))?;
    let modified_ns = stat.modified()?.duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(Snapshot {
        path: path.to_path_buf(),
        size: stat.len(),
        modified_ns,
        sha256: fingerprint.then(|| hash_file(path)).transpose()?,
    })
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

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
    let (writable, write_reason) = write_capability(fmt, &file);
    Ok(Track {
        id: TrackId::UNINDEXED,
        snapshot: snapshot(path, false)?,
        format: fmt,
        raw,
        metadata,
        diagnostics,
        writable,
        write_reason,
    })
}

fn write_capability(fmt: AudioFormat, file: &lofty::file::TaggedFile) -> (bool, String) {
    let required = match fmt {
        AudioFormat::Mp3 => TagType::Id3v2,
        AudioFormat::Flac => TagType::VorbisComments,
        AudioFormat::Mp4 => TagType::Mp4Ilst,
        _ => {
            return (
                false,
                "Read only: round-trip write adapter is not verified".into(),
            );
        }
    };
    if file.tags().iter().any(|t| t.tag_type() != required) {
        return (
            false,
            "Read only: multiple or unsupported tag containers".into(),
        );
    }
    if file.tags().iter().any(|t| t.has_format_specific_items()) {
        return (
            false,
            "Read only: format-specific metadata cannot be verified".into(),
        );
    }
    if file.tag(required).is_none() {
        return (false, "Read only: expected tag container is missing".into());
    }
    (true, "Verified adapter: targeted fields only".into())
}

fn item_key(field: Field) -> ItemKey {
    match field {
        Field::Title => ItemKey::TrackTitle,
        Field::Artist => ItemKey::TrackArtist,
        Field::AlbumArtist => ItemKey::AlbumArtist,
        Field::Album => ItemKey::AlbumTitle,
        Field::Track => ItemKey::TrackNumber,
        Field::Disc => ItemKey::DiscNumber,
        Field::Date => ItemKey::RecordingDate,
        Field::Artists => ItemKey::TrackArtists,
        Field::Genres => ItemKey::Genre,
    }
}

fn current_value(metadata: &Metadata, field: Field) -> Option<FieldValue> {
    match field {
        Field::Track => metadata.track.clone().map(FieldValue::Number),
        Field::Disc => metadata.disc.clone().map(FieldValue::Number),
        Field::Artists => Some(FieldValue::TextList(metadata.artists.clone())),
        Field::Genres => Some(FieldValue::TextList(metadata.genres.clone())),
        Field::Date => metadata.date.clone().map(FieldValue::Date),
        _ => metadata.value(field).map(FieldValue::Text),
    }
}

fn resulting_values(
    metadata: &Metadata,
    edits: &[EditOperation],
) -> Result<Vec<(Field, Option<FieldValue>)>> {
    let mut values = Vec::<(Field, Option<FieldValue>)>::new();
    for edit in edits {
        edit.validate().map_err(anyhow::Error::msg)?;
        let field = edit.field();
        let slot = if let Some(index) = values.iter().position(|(f, _)| *f == field) {
            &mut values[index].1
        } else {
            values.push((field, current_value(metadata, field)));
            &mut values.last_mut().expect("just pushed field").1
        };
        match edit {
            EditOperation::Set { value, .. } => *slot = Some(value.clone()),
            EditOperation::Clear { .. } => *slot = None,
            EditOperation::AddValue { value, .. } => {
                let list = slot.get_or_insert_with(|| FieldValue::TextList(Vec::new()));
                if let FieldValue::TextList(list) = list {
                    if !list.contains(value) {
                        list.push(value.clone());
                    }
                } else {
                    bail!("invalid list state for {}", field.label());
                }
            }
            EditOperation::RemoveValue { value, .. } => {
                if let Some(FieldValue::TextList(list)) = slot {
                    list.retain(|item| item != value);
                }
            }
        }
    }
    Ok(values)
}

pub fn validate_operation_for_format(format: AudioFormat, edit: &EditOperation) -> Result<()> {
    edit.validate().map_err(anyhow::Error::msg)?;
    if matches!(edit.field(), Field::Artists | Field::Genres) && format != AudioFormat::Flac {
        bail!(
            "list field {} is currently writable only in FLAC",
            edit.field().label()
        );
    }
    Ok(())
}

pub fn write_to_temp(source: &Path, temp: &Path, edits: &[EditOperation]) -> Result<()> {
    let original = read_track(source)?;
    if !original.writable {
        bail!("{}", original.write_reason);
    }
    for edit in edits {
        validate_operation_for_format(original.format, edit)?;
    }
    let values = resulting_values(&original.metadata, edits)?;
    fs::copy(source, temp)?;
    let mut tagged = read_from_path(temp)?;
    let tag_type = match original.format {
        AudioFormat::Mp3 => TagType::Id3v2,
        AudioFormat::Flac => TagType::VorbisComments,
        AudioFormat::Mp4 => TagType::Mp4Ilst,
        _ => bail!("unsupported write format"),
    };
    let tag = tagged.tag_mut(tag_type).context("tag disappeared")?;
    for (field, value) in &values {
        let key = item_key(*field);
        tag.remove_key(&key);
        if matches!(field, Field::Track | Field::Disc) {
            if let Some(FieldValue::Number(pair)) = value {
                if tag_type == TagType::Mp4Ilst {
                    let total_key = if *field == Field::Track {
                        ItemKey::TrackTotal
                    } else {
                        ItemKey::DiscTotal
                    };
                    tag.remove_key(&total_key);
                    if let Some(total) = pair.total
                        && !tag.insert_text(total_key, total.to_string())
                    {
                        bail!("tag container cannot store {} total", field.label());
                    }
                }
            } else if tag_type == TagType::Mp4Ilst {
                tag.remove_key(&if *field == Field::Track {
                    ItemKey::TrackTotal
                } else {
                    ItemKey::DiscTotal
                });
            }
        }
        let texts = match value {
            None => Vec::new(),
            Some(FieldValue::Text(text) | FieldValue::Date(text)) => vec![text.clone()],
            Some(FieldValue::Number(pair)) => vec![if tag_type == TagType::Mp4Ilst {
                pair.number.to_string()
            } else {
                match pair.total {
                    Some(total) => format!("{}/{total}", pair.number),
                    None => pair.number.to_string(),
                }
            }],
            Some(FieldValue::TextList(list)) => list.clone(),
        };
        for text in texts {
            if !tag.push(TagItem::new(key.clone(), ItemValue::Text(text))) {
                bail!("tag container cannot store {}", field.label());
            }
        }
    }
    let mut output = OpenOptions::new().read(true).write(true).open(temp)?;
    tagged.save_to(&mut output, WriteOptions::default())?;
    output.sync_all()?;
    let reread = read_track(temp)?;
    for (field, expected) in &values {
        let actual = current_value(&reread.metadata, *field);
        let empty = |value: &Option<FieldValue>| match value {
            None => true,
            Some(FieldValue::TextList(list)) => list.is_empty(),
            _ => false,
        };
        let matches = match expected {
            None => empty(&actual),
            Some(FieldValue::TextList(list)) if list.is_empty() => empty(&actual),
            _ => actual == *expected,
        };
        if !matches {
            bail!("written {} did not verify", field.label());
        }
    }
    let mut edited_keys: Vec<_> = values
        .iter()
        .map(|(field, _)| format!("{:?}", item_key(*field)))
        .collect();
    for (field, _) in &values {
        match field {
            Field::Track => edited_keys.push(format!("{:?}", ItemKey::TrackTotal)),
            Field::Disc => edited_keys.push(format!("{:?}", ItemKey::DiscTotal)),
            _ => {}
        }
    }
    let untouched = |track: &Track| {
        let mut entries: Vec<_> = track
            .raw
            .iter()
            .filter(|t| !edited_keys.contains(&t.key))
            .cloned()
            .collect();
        for entry in &mut entries {
            entry.order = 0;
        }
        entries.sort_by(|a, b| {
            (
                &a.container,
                &a.native_key,
                &a.key,
                format!("{:?}", a.value),
            )
                .cmp(&(
                    &b.container,
                    &b.native_key,
                    &b.key,
                    format!("{:?}", b.value),
                ))
        });
        entries
    };
    if untouched(&original) != untouched(&reread) {
        bail!("untouched metadata changed; write rejected");
    }
    if audio_hash(source)? != audio_hash(temp)? {
        bail!("audio payload changed; write rejected");
    }
    Ok(())
}

pub fn audio_hash(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let (start, end) = match format(path) {
        Some(AudioFormat::Mp3) => {
            let mut header = [0; 10];
            file.read_exact(&mut header)?;
            let start = if &header[..3] == b"ID3" {
                10 + u64::from(header[6]) * 2_097_152
                    + u64::from(header[7]) * 16_384
                    + u64::from(header[8]) * 128
                    + u64::from(header[9])
            } else {
                0
            };
            let mut end = len;
            if len >= 128 {
                file.seek(SeekFrom::End(-128))?;
                let mut marker = [0; 3];
                file.read_exact(&mut marker)?;
                if &marker == b"TAG" {
                    end -= 128;
                }
            }
            (start, end)
        }
        Some(AudioFormat::Flac) => {
            let mut marker = [0; 4];
            file.read_exact(&mut marker)?;
            if &marker != b"fLaC" {
                bail!("invalid FLAC stream");
            }
            let mut offset = 4u64;
            loop {
                let mut header = [0; 4];
                file.read_exact(&mut header)?;
                offset += 4
                    + u64::from(header[1]) * 65_536
                    + u64::from(header[2]) * 256
                    + u64::from(header[3]);
                if header[0] & 0x80 != 0 {
                    break;
                }
                file.seek(SeekFrom::Start(offset))?;
            }
            (offset, len)
        }
        Some(AudioFormat::Mp4) => return mp4_audio_hash(&mut file),
        _ => bail!("audio comparison unavailable for this format"),
    };
    if start > end {
        bail!("invalid audio boundaries");
    }
    file.seek(SeekFrom::Start(start))?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file.take(end - start), &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn mp4_audio_hash(file: &mut File) -> Result<String> {
    let len = file.metadata()?.len();
    let mut offset = 0u64;
    let mut found = false;
    let mut hash = Sha256::new();
    while offset < len {
        if len - offset < 8 {
            bail!("truncated MP4 atom header");
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header)?;
        let short_size = u32::from_be_bytes(header[..4].try_into()?);
        let header_size = if short_size == 1 { 16 } else { 8 };
        let size = match short_size {
            0 => len - offset,
            1 => {
                let mut extended = [0u8; 8];
                file.read_exact(&mut extended)?;
                u64::from_be_bytes(extended)
            }
            other => u64::from(other),
        };
        if size < header_size || size > len - offset {
            bail!("invalid MP4 atom size");
        }
        if &header[4..8] == b"mdat" {
            found = true;
            hash.update((size - header_size).to_be_bytes());
            std::io::copy(&mut file.take(size - header_size), &mut hash)?;
        }
        offset += size;
    }
    if !found {
        bail!("MP4 has no mdat atom");
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_roundtrip_retains_audio_and_unrelated_tags() {
        for extension in ["mp3", "flac"] {
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("tests/fixtures/problem.{extension}"));
            let original = read_track(&source).expect("fixture reads");
            assert!(
                original.writable,
                "{}: {}",
                source.display(),
                original.write_reason
            );
            let before_audio = audio_hash(&source).expect("audio hash");
            let dir = tempfile::tempdir().expect("tempdir");
            let output = dir.path().join(format!("changed.{extension}"));
            write_to_temp(
                &source,
                &output,
                &[EditOperation::Set {
                    field: Field::AlbumArtist,
                    value: FieldValue::Text("Corrected Artist".into()),
                }],
            )
            .expect("safe write");
            let changed = read_track(&output).expect("written tags read");
            assert_eq!(
                changed.metadata.album_artist.as_deref(),
                Some("Corrected Artist")
            );
            assert_eq!(before_audio, audio_hash(&output).expect("audio hash"));
            assert!(
                changed
                    .raw
                    .iter()
                    .any(|r| matches!(&r.value, RawValue::Text(v) if v == "keep this comment"))
            );
            assert!(
                changed
                    .raw
                    .iter()
                    .any(|r| matches!(&r.value, RawValue::Text(v) if v == "untouched"))
            );
            if extension == "flac" {
                assert_eq!(
                    changed
                        .raw
                        .iter()
                        .filter(|r| r.native_key.eq_ignore_ascii_case("ARTISTS"))
                        .count(),
                    2
                );
                assert!(
                    changed
                        .raw
                        .iter()
                        .any(|r| r.native_key.eq_ignore_ascii_case("X_CUSTOM"))
                );
            }
        }
    }

    #[test]
    fn exposes_repeated_raw_artists_and_read_only_formats() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let flac = read_track(&root.join("problem.flac")).expect("flac fixture");
        assert_eq!(
            flac.raw
                .iter()
                .filter(|r| r.native_key.eq_ignore_ascii_case("ARTISTS"))
                .count(),
            2
        );
        assert_eq!(flac.metadata.artists.len(), 2);
        for extension in ["ogg", "opus"] {
            let track =
                read_track(&root.join(format!("readonly.{extension}"))).expect("read-only fixture");
            assert!(!track.writable);
            assert!(track.metadata.title.is_some());
        }
    }

    #[test]
    fn m4a_roundtrip_preserves_audio_artwork_and_unrelated_metadata() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/roundtrip.m4a");
        let before = read_track(&source).expect("read fixture");
        assert!(before.writable, "{}", before.write_reason);
        assert!(before.metadata.artwork_count > 0);
        let before_audio = audio_hash(&source).expect("audio hash");
        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join("changed.m4a");
        write_to_temp(
            &source,
            &output,
            &[EditOperation::Set {
                field: Field::AlbumArtist,
                value: FieldValue::Text("Corrected Artist".into()),
            }],
        )
        .expect("safe write");
        let after = read_track(&output).expect("read result");
        assert_eq!(
            after.metadata.album_artist.as_deref(),
            Some("Corrected Artist")
        );
        assert_eq!(before_audio, audio_hash(&output).expect("audio hash"));
        assert_eq!(before.metadata.artwork_count, after.metadata.artwork_count);
        assert!(after.raw.iter().any(
            |item| matches!(&item.value, RawValue::Text(value) if value == "keep this comment")
        ));
    }

    #[test]
    fn m4a_number_pair_roundtrip() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/roundtrip.m4a");
        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join("numbered.m4a");
        write_to_temp(
            &source,
            &output,
            &[EditOperation::Set {
                field: Field::Track,
                value: FieldValue::Number(parse_number("3/12").unwrap()),
            }],
        )
        .expect("write pair");
        let after = read_track(&output).expect("read result");
        assert_eq!(after.metadata.value(Field::Track).as_deref(), Some("3/12"));
    }

    #[test]
    fn m4a_clear_removes_number_and_total() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/roundtrip.m4a");
        let dir = tempfile::tempdir().unwrap();
        let numbered = dir.path().join("numbered.m4a");
        let cleared = dir.path().join("cleared.m4a");
        write_to_temp(
            &source,
            &numbered,
            &[EditOperation::Set {
                field: Field::Track,
                value: FieldValue::Number(parse_number("3/12").unwrap()),
            }],
        )
        .unwrap();
        write_to_temp(
            &numbered,
            &cleared,
            &[EditOperation::Clear {
                field: Field::Track,
            }],
        )
        .unwrap();
        assert_eq!(read_track(&cleared).unwrap().metadata.track, None);
    }

    #[test]
    fn clear_and_list_operations_roundtrip() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.flac");
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("changed.flac");
        write_to_temp(
            &source,
            &output,
            &[
                EditOperation::Clear {
                    field: Field::AlbumArtist,
                },
                EditOperation::Set {
                    field: Field::Genres,
                    value: FieldValue::TextList(vec!["Jazz".into()]),
                },
                EditOperation::AddValue {
                    field: Field::Genres,
                    value: "Blues".into(),
                },
                EditOperation::RemoveValue {
                    field: Field::Genres,
                    value: "Jazz".into(),
                },
            ],
        )
        .unwrap();
        let after = read_track(&output).unwrap();
        assert_eq!(after.metadata.album_artist, None);
        assert_eq!(after.metadata.genres, vec!["Blues"]);
    }

    #[test]
    fn artists_and_date_roundtrip() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.flac");
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("changed.flac");
        write_to_temp(
            &source,
            &output,
            &[
                EditOperation::Set {
                    field: Field::Artists,
                    value: FieldValue::TextList(vec!["One".into(), "Two".into()]),
                },
                EditOperation::Set {
                    field: Field::Date,
                    value: FieldValue::Date("2024".into()),
                },
            ],
        )
        .unwrap();
        let after = read_track(&output).unwrap();
        assert_eq!(after.metadata.artists, vec!["One", "Two"]);
        assert_eq!(after.metadata.date.as_deref(), Some("2024"));
    }

    #[test]
    fn unsupported_list_format_is_rejected_before_write() {
        let edit = EditOperation::AddValue {
            field: Field::Artists,
            value: "Two".into(),
        };
        assert!(
            validate_operation_for_format(AudioFormat::Mp3, &edit)
                .unwrap_err()
                .to_string()
                .contains("only in FLAC")
        );
    }
}
