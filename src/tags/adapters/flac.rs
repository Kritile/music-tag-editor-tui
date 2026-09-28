use super::TagAdapter;
use crate::tags::verify::hash_range;
use anyhow::{Result, bail};
use lofty::tag::TagType;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub(super) struct Flac;

impl TagAdapter for Flac {
    fn tag_type(&self) -> Option<TagType> {
        Some(TagType::VorbisComments)
    }
    fn supports_lists(&self) -> bool {
        true
    }

    fn audio_hash(&self, path: &Path) -> Result<String> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
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
        hash_range(&mut file, offset, len)
    }
}
