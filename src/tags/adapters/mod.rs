mod flac;
mod mp3;
mod mp4;
mod ogg;
mod opus;

use crate::domain::{AudioFormat, EditOperation, Field, NumberPair};
use anyhow::{Result, bail};
use lofty::tag::{ItemKey, Tag, TagType};
use std::path::Path;

use super::capabilities::{WriteCapabilities, write_capabilities};

pub(super) trait TagAdapter: Sync {
    fn tag_type(&self) -> Option<TagType>;

    fn capabilities(&self, file: &lofty::file::TaggedFile) -> WriteCapabilities {
        write_capabilities(file, self.tag_type())
    }

    fn validate_operation(&self, edit: &EditOperation) -> Result<()> {
        edit.validate().map_err(anyhow::Error::msg)?;
        let single_genre = match edit {
            EditOperation::Clear {
                field: Field::Genres,
            } => true,
            EditOperation::Set {
                field: Field::Genres,
                value: crate::domain::FieldValue::TextList(values),
            } => values.len() <= 1,
            _ => false,
        };
        if matches!(edit.field(), Field::Artists | Field::Genres)
            && !self.supports_lists()
            && !single_genre
        {
            bail!(
                "list field {} is currently writable only in FLAC",
                edit.field().label()
            );
        }
        Ok(())
    }

    fn supports_lists(&self) -> bool {
        false
    }

    fn number_text(
        &self,
        _tag: &mut Tag,
        _field: Field,
        pair: Option<&NumberPair>,
    ) -> Result<Option<String>> {
        Ok(pair.map(|pair| match pair.total {
            Some(total) => format!("{}/{total}", pair.number),
            None => pair.number.to_string(),
        }))
    }

    fn audio_hash(&self, _path: &Path) -> Result<String> {
        bail!("audio comparison unavailable for this format")
    }
}

pub(super) fn for_format(format: AudioFormat) -> &'static dyn TagAdapter {
    match format {
        AudioFormat::Mp3 => &mp3::Mp3,
        AudioFormat::Flac => &flac::Flac,
        AudioFormat::Mp4 => &mp4::Mp4,
        AudioFormat::OggVorbis => &ogg::Ogg,
        AudioFormat::Opus => &opus::Opus,
    }
}

pub(super) fn total_key(field: Field) -> Option<ItemKey> {
    match field {
        Field::Track => Some(ItemKey::TrackTotal),
        Field::Disc => Some(ItemKey::DiscTotal),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_format_selects_its_expected_tag_container() {
        assert_eq!(
            for_format(AudioFormat::Mp3).tag_type(),
            Some(TagType::Id3v2)
        );
        assert_eq!(
            for_format(AudioFormat::Flac).tag_type(),
            Some(TagType::VorbisComments)
        );
        assert_eq!(
            for_format(AudioFormat::Mp4).tag_type(),
            Some(TagType::Mp4Ilst)
        );
        assert_eq!(for_format(AudioFormat::OggVorbis).tag_type(), None);
        assert_eq!(for_format(AudioFormat::Opus).tag_type(), None);
    }
}
