use super::reader::read_track;
use crate::domain::{AudioFormat, EditOperation, Field, FieldValue, Metadata};
use anyhow::{Context, Result, bail};
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::read_from_path;
use lofty::tag::{ItemKey, ItemValue, TagItem};
use std::fs::{self, OpenOptions};
use std::path::Path;

pub(super) fn item_key(field: Field) -> ItemKey {
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

pub(super) fn current_value(metadata: &Metadata, field: Field) -> Option<FieldValue> {
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
    super::adapters::for_format(format).validate_operation(edit)
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
    let adapter = super::adapters::for_format(original.format);
    let tag_type = adapter.tag_type().context("unsupported write format")?;
    let tag = tagged.tag_mut(tag_type).context("tag disappeared")?;
    for (field, value) in &values {
        let key = item_key(*field);
        tag.remove_key(&key);
        let texts = match value {
            None if matches!(field, Field::Track | Field::Disc) => {
                adapter.number_text(tag, *field, None)?;
                Vec::new()
            }
            None => Vec::new(),
            Some(FieldValue::Text(text) | FieldValue::Date(text)) => vec![text.clone()],
            Some(FieldValue::Number(pair)) => adapter
                .number_text(tag, *field, Some(pair))?
                .into_iter()
                .collect(),
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
    super::verify::verify_write(source, temp, &original, &values)?;
    Ok(())
}
