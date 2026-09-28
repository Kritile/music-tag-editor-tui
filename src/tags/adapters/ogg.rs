use super::TagAdapter;
use lofty::tag::TagType;

pub(super) struct Ogg;

impl TagAdapter for Ogg {
    fn tag_type(&self) -> Option<TagType> {
        None
    }
}
