#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = music_tag_editor::changes::parse_batch(data);
});
