use music_tag_editor::domain::{AudioFormat, TrackId};

#[test]
fn audio_format_recognizes_extensions_and_preserves_stored_names() {
    assert_eq!(AudioFormat::from_extension("M4A"), Some(AudioFormat::Mp4));
    assert_eq!(
        AudioFormat::from_extension("oga"),
        Some(AudioFormat::OggVorbis)
    );
    assert_eq!(AudioFormat::from_extension("wav"), None);
    for (format, stored) in [
        (AudioFormat::Mp3, "MP3"),
        (AudioFormat::Flac, "FLAC"),
        (AudioFormat::Mp4, "MP4"),
        (AudioFormat::OggVorbis, "Ogg"),
        (AudioFormat::Opus, "Opus"),
    ] {
        let json = format!("\"{stored}\"");
        assert_eq!(serde_json::to_string(&format).unwrap(), json);
        assert_eq!(serde_json::from_str::<AudioFormat>(&json).unwrap(), format);
    }
}

#[test]
fn track_id_rejects_invalid_values_without_changing_json_shape() {
    assert!(TrackId::indexed(0).is_none());
    assert!(TrackId::indexed(-1).is_none());
    let id = TrackId::indexed(42).expect("positive id");
    assert_eq!(id.get(), 42);
    assert_eq!(serde_json::to_string(&id).unwrap(), "42");
    assert_eq!(serde_json::from_str::<TrackId>("42").unwrap(), id);
    assert_eq!(
        serde_json::from_str::<TrackId>("0").unwrap(),
        TrackId::UNINDEXED
    );
    assert!(serde_json::from_str::<TrackId>("-1").is_err());
}
