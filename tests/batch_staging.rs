use music_tag_editor::changes;
use music_tag_editor::domain::{EditOperation, Field, FieldValue};
use music_tag_editor::tags;
use std::fs;
use std::path::Path;

#[test]
fn stages_multiple_fields_and_files_without_touching_audio() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.flac");
    let paths: Vec<_> = ["a.flac", "b.flac"]
        .iter()
        .map(|name| {
            let path = dir.path().join(name);
            fs::copy(&source, &path).expect("fixture copy");
            path
        })
        .collect();
    let tracks: Vec<_> = paths
        .iter()
        .map(|path| tags::read_track(path).expect("read"))
        .collect();
    let before: Vec<_> = paths
        .iter()
        .map(|path| fs::read(path).expect("bytes"))
        .collect();
    let edits = [
        EditOperation::Set {
            field: Field::Album,
            value: FieldValue::Text("New Album".into()),
        },
        EditOperation::Clear { field: Field::Date },
    ];
    let staged = changes::stage_operations(dir.path(), &tracks, &edits).expect("stage batch");
    assert_eq!(staged.len(), 2);
    for (path, original) in paths.iter().zip(before) {
        assert_eq!(fs::read(path).expect("still unchanged"), original);
    }
    assert!(staged.iter().all(|pending| pending.edits == edits));
    assert!(staged.iter().all(|pending| pending.before.len() == 2));
}

#[test]
fn batch_validation_failure_does_not_replace_existing_staging() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.flac");
    let path = dir.path().join("a.flac");
    fs::copy(source, &path).expect("fixture copy");
    let track = tags::read_track(&path).expect("read");
    let edit = EditOperation::Clear {
        field: Field::Title,
    };
    let initial = changes::stage_operations(
        dir.path(),
        std::slice::from_ref(&track),
        std::slice::from_ref(&edit),
    )
    .expect("initial staging");
    let mut invalid = track.clone();
    invalid.writable = false;
    assert!(
        changes::stage_operations(
            dir.path(),
            &[track, invalid],
            &[EditOperation::Clear {
                field: Field::Album
            }]
        )
        .is_err()
    );
    let after = changes::load_staged(dir.path()).expect("unchanged staging");
    assert_eq!(after.len(), initial.len());
    assert_eq!(after[0].edits, initial[0].edits);
}
