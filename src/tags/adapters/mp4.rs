use super::{TagAdapter, total_key};
use crate::domain::{Field, NumberPair};
use anyhow::{Result, bail};
use lofty::tag::{Tag, TagType};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub(super) struct Mp4;

impl TagAdapter for Mp4 {
    fn tag_type(&self) -> Option<TagType> {
        Some(TagType::Mp4Ilst)
    }

    fn number_text(
        &self,
        tag: &mut Tag,
        field: Field,
        pair: Option<&NumberPair>,
    ) -> Result<Option<String>> {
        if let Some(key) = total_key(field) {
            tag.remove_key(&key);
            if let Some(total) = pair.and_then(|pair| pair.total)
                && !tag.insert_text(key, total.to_string())
            {
                bail!("tag container cannot store {} total", field.label());
            }
        }
        Ok(pair.map(|pair| pair.number.to_string()))
    }

    fn audio_hash(&self, path: &Path) -> Result<String> {
        let mut file = File::open(path)?;
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
                std::io::copy(&mut file.by_ref().take(size - header_size), &mut hash)?;
            }
            offset += size;
        }
        if !found {
            bail!("MP4 has no mdat atom");
        }
        Ok(format!("{:x}", hash.finalize()))
    }
}
