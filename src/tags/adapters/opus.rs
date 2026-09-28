use super::TagAdapter;
use lofty::tag::TagType;

pub(super) struct Opus;

impl TagAdapter for Opus {
    fn tag_type(&self) -> Option<TagType> {
        None
    }
}
