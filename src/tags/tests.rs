use super::*;
use crate::domain::{AudioFormat, EditOperation, Field, FieldValue, RawValue, parse_number};
use std::path::Path;

#[test]
fn unsupported_audio_path_has_typed_error() {
    let path = Path::new("/synthetic/song.wav");
    assert!(
        matches!(read_track(path), Err(TagError::UnsupportedFormat { path: found }) if found == path)
    );
}

#[test]
fn verified_roundtrip_retains_audio_and_unrelated_tags() {
    for extension in ["mp3", "flac"] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/problem.{extension}"));
        let original = read_track(&source).expect("fixture reads");
        assert!(
            original.writable,
            "{}: {}",
            source.display(),
            original.write_reason
        );
        let before_audio = audio_hash(&source).expect("audio hash");
        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join(format!("changed.{extension}"));
        write_to_temp(
            &source,
            &output,
            &[EditOperation::Set {
                field: Field::AlbumArtist,
                value: FieldValue::Text("Corrected Artist".into()),
            }],
        )
        .expect("safe write");
        let changed = read_track(&output).expect("written tags read");
        assert_eq!(
            changed.metadata.album_artist.as_deref(),
            Some("Corrected Artist")
        );
        assert_eq!(before_audio, audio_hash(&output).expect("audio hash"));
        assert!(
            changed
                .raw
                .iter()
                .any(|r| matches!(&r.value, RawValue::Text(v) if v == "keep this comment"))
        );
        assert!(
            changed
                .raw
                .iter()
                .any(|r| matches!(&r.value, RawValue::Text(v) if v == "untouched"))
        );
        if extension == "flac" {
            assert_eq!(
                changed
                    .raw
                    .iter()
                    .filter(|r| r.native_key.eq_ignore_ascii_case("ARTISTS"))
                    .count(),
                2
            );
            assert!(
                changed
                    .raw
                    .iter()
                    .any(|r| r.native_key.eq_ignore_ascii_case("X_CUSTOM"))
            );
        }
    }
}

#[test]
fn exposes_repeated_raw_artists_and_read_only_formats() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let flac = read_track(&root.join("problem.flac")).expect("flac fixture");
    assert_eq!(
        flac.raw
            .iter()
            .filter(|r| r.native_key.eq_ignore_ascii_case("ARTISTS"))
            .count(),
        2
    );
    assert_eq!(flac.metadata.artists.len(), 2);
    for extension in ["ogg", "opus"] {
        let track =
            read_track(&root.join(format!("readonly.{extension}"))).expect("read-only fixture");
        assert!(!track.writable);
        assert!(track.metadata.title.is_some());
    }
}

#[test]
fn m4a_roundtrip_preserves_audio_artwork_and_unrelated_metadata() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/roundtrip.m4a");
    let before = read_track(&source).expect("read fixture");
    assert!(before.writable, "{}", before.write_reason);
    assert!(before.metadata.artwork_count > 0);
    let before_audio = audio_hash(&source).expect("audio hash");
    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("changed.m4a");
    write_to_temp(
        &source,
        &output,
        &[EditOperation::Set {
            field: Field::AlbumArtist,
            value: FieldValue::Text("Corrected Artist".into()),
        }],
    )
    .expect("safe write");
    let after = read_track(&output).expect("read result");
    assert_eq!(
        after.metadata.album_artist.as_deref(),
        Some("Corrected Artist")
    );
    assert_eq!(before_audio, audio_hash(&output).expect("audio hash"));
    assert_eq!(before.metadata.artwork_count, after.metadata.artwork_count);
    assert!(
        after.raw.iter().any(
            |item| matches!(&item.value, RawValue::Text(value) if value == "keep this comment")
        )
    );
}

#[test]
fn m4a_number_pair_roundtrip() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/roundtrip.m4a");
    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("numbered.m4a");
    write_to_temp(
        &source,
        &output,
        &[EditOperation::Set {
            field: Field::Track,
            value: FieldValue::Number(parse_number("3/12").unwrap()),
        }],
    )
    .expect("write pair");
    let after = read_track(&output).expect("read result");
    assert_eq!(after.metadata.value(Field::Track).as_deref(), Some("3/12"));
}

#[test]
fn m4a_clear_removes_number_and_total() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/roundtrip.m4a");
    let dir = tempfile::tempdir().unwrap();
    let numbered = dir.path().join("numbered.m4a");
    let cleared = dir.path().join("cleared.m4a");
    write_to_temp(
        &source,
        &numbered,
        &[EditOperation::Set {
            field: Field::Track,
            value: FieldValue::Number(parse_number("3/12").unwrap()),
        }],
    )
    .unwrap();
    write_to_temp(
        &numbered,
        &cleared,
        &[EditOperation::Clear {
            field: Field::Track,
        }],
    )
    .unwrap();
    assert_eq!(read_track(&cleared).unwrap().metadata.track, None);
}

#[test]
fn clear_and_list_operations_roundtrip() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.flac");
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("changed.flac");
    write_to_temp(
        &source,
        &output,
        &[
            EditOperation::Clear {
                field: Field::AlbumArtist,
            },
            EditOperation::Set {
                field: Field::Genres,
                value: FieldValue::TextList(vec!["Jazz".into()]),
            },
            EditOperation::AddValue {
                field: Field::Genres,
                value: "Blues".into(),
            },
            EditOperation::RemoveValue {
                field: Field::Genres,
                value: "Jazz".into(),
            },
        ],
    )
    .unwrap();
    let after = read_track(&output).unwrap();
    assert_eq!(after.metadata.album_artist, None);
    assert_eq!(after.metadata.genres, vec!["Blues"]);
}

#[test]
fn artists_and_date_roundtrip() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.flac");
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("changed.flac");
    write_to_temp(
        &source,
        &output,
        &[
            EditOperation::Set {
                field: Field::Artists,
                value: FieldValue::TextList(vec!["One".into(), "Two".into()]),
            },
            EditOperation::Set {
                field: Field::Date,
                value: FieldValue::Date("2024".into()),
            },
        ],
    )
    .unwrap();
    let after = read_track(&output).unwrap();
    assert_eq!(after.metadata.artists, vec!["One", "Two"]);
    assert_eq!(after.metadata.date.as_deref(), Some("2024"));
}

#[test]
fn unsupported_list_format_is_rejected_before_write() {
    let edit = EditOperation::AddValue {
        field: Field::Artists,
        value: "Two".into(),
    };
    assert!(
        validate_operation_for_format(AudioFormat::Mp3, &edit)
            .unwrap_err()
            .to_string()
            .contains("only in FLAC")
    );
}
