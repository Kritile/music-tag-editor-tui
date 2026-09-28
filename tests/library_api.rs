use music_tag_editor::{domain::Field, rules, tags};
use std::path::Path;

#[test]
fn external_client_can_read_and_inspect_tracks() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3");
    let track = tags::read_track(&fixture).expect("read fixture through library API");

    assert!(track.metadata.value(Field::Title).is_some());
    let issues = rules::inspect(&[track], true);
    assert!(!issues.is_empty());
}
