use super::TagAdapter;
use crate::tags::verify::hash_range;
use anyhow::Result;
use lofty::tag::TagType;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub(super) struct Mp3;

impl TagAdapter for Mp3 {
    fn tag_type(&self) -> Option<TagType> {
        Some(TagType::Id3v2)
    }

    fn audio_hash(&self, path: &Path) -> Result<String> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
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
        hash_range(&mut file, start, end)
    }
}
