use lofty::{
    config::WriteOptions,
    file::{AudioFile, TaggedFileExt},
    probe::read_from_path,
    tag::{ItemKey, ItemValue, TagItem},
};
use music_tag_editor::{
    domain::{EditOperation, Field, FieldValue, RawTag},
    tags::{self, TagError},
};
use std::path::Path;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn untouched_tags(raw: &[RawTag]) -> Vec<String> {
    let mut tags: Vec<_> = raw
        .iter()
        .filter(|tag| tag.key != "AlbumArtist")
        .map(|tag| {
            let mut tag = tag.clone();
            tag.order = 0;
            serde_json::to_string(&tag).expect("raw tag JSON")
        })
        .collect();
    tags.sort();
    tags
}

fn source_with_artwork(name: &str, dir: &Path) -> std::path::PathBuf {
    let source = dir.join(name);
    std::fs::copy(fixture(name), &source).expect("copy fixture");
    if name.ends_with(".m4a") {
        let mut tagged = read_from_path(&source).expect("read MP4 fixture");
        tagged
            .primary_tag_mut()
            .expect("primary tag")
            .push_unchecked(TagItem::new(
                ItemKey::Unknown("----:com.apple.iTunes:X_CONTRACT".into()),
                ItemValue::Text("untouched".into()),
            ));
        tagged
            .save_to_path(&source, WriteOptions::default())
            .expect("add unknown metadata");
    } else {
        let cover = read_from_path(fixture("roundtrip.m4a"))
            .expect("read cover fixture")
            .primary_tag()
            .expect("MP4 primary tag")
            .pictures()[0]
            .clone();
        let mut tagged = read_from_path(&source).expect("read writable fixture");
        tagged
            .primary_tag_mut()
            .expect("primary tag")
            .push_picture(cover);
        tagged
            .save_to_path(&source, WriteOptions::default())
            .expect("add fixture artwork");
    }
    source
}

#[test]
fn writable_adapters_share_read_edit_verify_contract() {
    for (name, extension) in [
        ("problem.mp3", "mp3"),
        ("problem.flac", "flac"),
        ("roundtrip.m4a", "m4a"),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = source_with_artwork(name, dir.path());
        let before = tags::read_track(&source).expect("read source");
        assert!(before.writable, "{name}: {}", before.write_reason);
        assert!(
            before.metadata.artwork_count > 0,
            "{name}: artwork fixture must be nonempty"
        );
        assert!(before.raw.iter().any(|tag| {
            matches!(&tag.value, music_tag_editor::domain::RawValue::Text(text) if text == "keep this comment")
        }), "{name}: fixture must include unrelated comment metadata");
        assert!(
            before
                .raw
                .iter()
                .any(|tag| tag.native_key.contains("X_CUSTOM")
                    || tag.native_key.contains("X_CONTRACT")),
            "{name}: fixture must include unknown metadata"
        );
        let original_audio = tags::audio_hash(&source).expect("audio payload");
        let output = dir.path().join(format!("changed.{extension}"));
        tags::write_to_temp(
            &source,
            &output,
            &[EditOperation::Set {
                field: Field::AlbumArtist,
                value: FieldValue::Text("Contract Artist".into()),
            }],
        )
        .expect("edit succeeds");
        let after = tags::read_track(&output).expect("reread succeeds");
        assert_eq!(
            after.metadata.album_artist.as_deref(),
            Some("Contract Artist"),
            "{name}"
        );
        assert_eq!(
            tags::audio_hash(&output).expect("written audio"),
            original_audio,
            "{name}"
        );
        assert_eq!(
            untouched_tags(&after.raw),
            untouched_tags(&before.raw),
            "{name}: unknown and unmodified metadata"
        );
        assert_eq!(
            after.metadata.artwork_count, before.metadata.artwork_count,
            "{name}: artwork count"
        );
    }
}

#[test]
fn read_only_adapters_reject_edits_without_creating_output() {
    for extension in ["ogg", "opus"] {
        let source = fixture(&format!("readonly.{extension}"));
        let before = tags::read_track(&source).expect("read source");
        assert!(!before.writable);
        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join(format!("changed.{extension}"));
        assert!(matches!(
            tags::write_to_temp(
                &source,
                &output,
                &[EditOperation::Set {
                    field: Field::Title,
                    value: FieldValue::Text("Changed".into()),
                }],
            ),
            Err(TagError::NotWritable { .. })
        ));
        assert!(!output.exists());
    }
}
