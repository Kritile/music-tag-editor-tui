//! Format adapters and verified tag I/O.

mod adapters;
mod capabilities;
mod reader;
mod snapshot;
mod verify;
mod writer;

pub use reader::read_track;
pub use snapshot::{format, hash_file, snapshot};
pub use verify::audio_hash;
pub use writer::{validate_operation_for_format, write_to_temp};

#[cfg(test)]
mod tests;
