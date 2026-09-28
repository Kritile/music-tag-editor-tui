//! Format adapters and verified tag I/O.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum TagError {
    #[error("unsupported audio extension: {path}", path = .path.display())]
    UnsupportedFormat { path: PathBuf },
    #[error("read tags: {path}: {source:#}", path = .path.display())]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
    #[error("{path}: {reason}", path = .path.display())]
    NotWritable { path: PathBuf, reason: String },
    #[error("write tags: {path}: {source:#}", path = .path.display())]
    WriteFailed {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
}

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
