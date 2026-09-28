use lofty::file::TaggedFileExt;
use lofty::tag::TagType;

pub(super) struct WriteCapabilities {
    pub writable: bool,
    pub reason: String,
}

pub(super) fn write_capabilities(
    file: &lofty::file::TaggedFile,
    required: Option<TagType>,
) -> WriteCapabilities {
    let Some(required) = required else {
        return WriteCapabilities {
            writable: false,
            reason: "Read only: round-trip write adapter is not verified".into(),
        };
    };
    let reason = if file.tags().iter().any(|tag| tag.tag_type() != required) {
        Some("Read only: multiple or unsupported tag containers")
    } else if file
        .tags()
        .iter()
        .any(|tag| tag.has_format_specific_items())
    {
        Some("Read only: format-specific metadata cannot be verified")
    } else if file.tag(required).is_none() {
        Some("Read only: expected tag container is missing")
    } else {
        None
    };
    match reason {
        Some(reason) => WriteCapabilities {
            writable: false,
            reason: reason.into(),
        },
        None => WriteCapabilities {
            writable: true,
            reason: "Verified adapter: targeted fields only".into(),
        },
    }
}
