#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Write;

fuzz_target!(|data: &[u8]| {
    let Some((&selector, bytes)) = data.split_first() else {
        return;
    };
    let extension = match selector % 3 {
        0 => ".mp3",
        1 => ".flac",
        _ => ".m4a",
    };
    let Ok(mut file) = tempfile::Builder::new().suffix(extension).tempfile() else {
        return;
    };
    if file.write_all(bytes).is_ok() {
        let _ = music_tag_editor::tags::read_track(file.path());
    }
});
