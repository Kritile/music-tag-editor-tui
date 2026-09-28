//! Types shared by tag adapters, the library index, rules, and user interfaces.

mod audio_format;
mod edit;
mod ids;
mod issue;
mod metadata;
mod raw_tag;
mod snapshot;
mod track;

pub use audio_format::AudioFormat;
pub use edit::{Edit, Field};
pub use ids::TrackId;
pub use issue::{Confidence, Issue};
pub use metadata::{Metadata, NumberPair, parse_number};
pub use raw_tag::{RawTag, RawValue};
pub use snapshot::Snapshot;
pub use track::Track;
