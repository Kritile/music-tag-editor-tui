use super::adapters::for_format;
use super::reader::read_track;
use super::snapshot::format;
use super::writer::{current_value, item_key};
use crate::domain::{Field, FieldValue, Track};
use anyhow::{Result, bail};
use lofty::tag::ItemKey;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub fn audio_hash(path: &Path) -> Result<String> {
    let format = format(path)
        .ok_or_else(|| anyhow::anyhow!("audio comparison unavailable for this format"))?;
    for_format(format).audio_hash(path)
}

pub(super) fn hash_range(file: &mut File, start: u64, end: u64) -> Result<String> {
    if start > end {
        bail!("invalid audio boundaries");
    }
    file.seek(SeekFrom::Start(start))?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file.take(end - start), &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

pub(super) fn verify_write(
    source: &Path,
    temp: &Path,
    original: &Track,
    values: &[(Field, Option<FieldValue>)],
) -> Result<()> {
    let reread = read_track(temp)?;
    for (field, expected) in values {
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
    for (field, _) in values {
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
    if untouched(original) != untouched(&reread) {
        bail!("untouched metadata changed; write rejected");
    }
    if audio_hash(source)? != audio_hash(temp)? {
        bail!("audio payload changed; write rejected");
    }
    Ok(())
}
