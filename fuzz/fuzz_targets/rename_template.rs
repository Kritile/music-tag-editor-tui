#![no_main]

use libfuzzer_sys::fuzz_target;
use music_tag_editor::domain::{AudioFormat, Metadata, Snapshot, Track, TrackId};
use std::path::PathBuf;

fuzz_target!(|data: &[u8]| {
    let Ok(template) = std::str::from_utf8(data) else {
        return;
    };
    let track = Track {
        id: TrackId::UNINDEXED,
        snapshot: Snapshot {
            path: PathBuf::from("/music/song.mp3"),
            size: 0,
            modified_ns: 0,
            sha256: None,
        },
        format: AudioFormat::Mp3,
        raw: vec![],
        metadata: Metadata {
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            ..Metadata::default()
        },
        diagnostics: vec![],
        writable: true,
        write_reason: String::new(),
    };
    let _ = music_tag_editor::rename::render_template(&track, template);
});
