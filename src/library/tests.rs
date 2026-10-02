use super::*;

fn fixture_track(root: &Path) -> Track {
    let path = root.join("song.mp3");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3"),
        &path,
    )
    .expect("fixture copy");
    tags::read_track(&path).expect("fixture tags")
}

fn legacy_database(path: &Path, track: &Track, data: &str) {
    let connection = Connection::open(path).expect("legacy database");
    connection
        .execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version(version) VALUES (1);
             CREATE TABLE tracks (
               id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, size INTEGER NOT NULL,
               modified_ns TEXT NOT NULL, generation INTEGER NOT NULL, data TEXT NOT NULL
             );
             CREATE INDEX tracks_generation ON tracks(generation);",
        )
        .expect("legacy schema");
    connection.execute(
            "INSERT INTO tracks(id,path,size,modified_ns,generation,data) VALUES (42,?1,?2,?3,7,?4)",
            params![track.snapshot.path.to_string_lossy(), track.snapshot.size as i64,
                track.snapshot.modified_ns.to_string(), data],
        ).expect("legacy row");
}

#[test]
fn migrates_legacy_index_preserving_identity_and_queryable_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut track = fixture_track(temp.path());
    track.metadata.genres = vec!["Rock".into()];
    let path = temp.path().join("index.sqlite");
    let data = serde_json::to_string(&track).expect("track json");
    legacy_database(&path, &track, &data);

    let mut index = Index::open_at(&path, temp.path()).expect("migrate legacy index");
    let summary = index
        .query_tracks(&TrackQuery::default())
        .expect("SQL summaries")
        .items;
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].id.get(), 42);
    assert_eq!(summary[0].format, track.format);
    assert_eq!(summary[0].title, track.metadata.title);
    assert_eq!(summary[0].modified_ns, track.snapshot.modified_ns);
    assert_eq!(
        index
            .query_tracks(&TrackQuery {
                genre: Some("rock"),
                ..TrackQuery::default()
            })
            .expect("migrated genre")
            .items
            .len(),
        1
    );
    assert_eq!(index.tracks().expect("full tracks")[0].id.get(), 42);
    let version: i64 = index
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("version");
    assert_eq!(version, 2);
    assert_eq!(
        index.scan(temp.path(), |_, _| true).expect("rescan").reused,
        1
    );
    drop(index);
    let reopened = Index::open_at(&path, temp.path()).expect("idempotent open");
    assert_eq!(
        reopened
            .query_tracks(&TrackQuery::default())
            .expect("summaries")
            .items[0]
            .id
            .get(),
        42
    );
}

#[test]
fn invalid_legacy_json_rolls_back_schema_upgrade() {
    let temp = tempfile::tempdir().expect("tempdir");
    let track = fixture_track(temp.path());
    let path = temp.path().join("index.sqlite");
    legacy_database(&path, &track, "{broken");
    assert!(matches!(
        Index::open_at(&path, temp.path()),
        Err(LibraryError::InvalidLegacyTrack { id: 42, .. })
    ));
    let connection = Connection::open(&path).expect("reopen old schema");
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("version");
    assert_eq!(version, 1);
    let old_time: String = connection
        .query_row("SELECT modified_ns FROM tracks WHERE id = 42", [], |row| {
            row.get(0)
        })
        .expect("old row intact");
    assert_eq!(old_time, track.snapshot.modified_ns.to_string());
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'tracks_v2'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .expect("rollback"),
        0
    );
}

#[test]
fn summaries_filter_and_sort_without_decoding_track_json() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut track = fixture_track(temp.path());
    track.metadata.title = Some("First Song".into());
    track.metadata.artist = Some("Solo".into());
    track.metadata.album_artist = Some("Various".into());
    track.metadata.album = Some("Collection".into());
    track.metadata.track = Some(crate::domain::NumberPair {
        number: 1,
        total: None,
    });
    track.metadata.genres = vec!["Rock".into(), "Jazz".into()];
    let mut index = Index::open_at(&temp.path().join("index.sqlite"), temp.path()).expect("index");
    let first_id = index.upsert_track(&track, 1).expect("upsert");
    let mut second = track.clone();
    second.snapshot.path = temp.path().join("second.mp3");
    second.metadata.title = Some("Earlier Alphabetically".into());
    second.metadata.track = Some(crate::domain::NumberPair {
        number: 2,
        total: None,
    });
    second.metadata.genres = vec!["Classical".into()];
    let second_id = index.upsert_track(&second, 1).expect("second track");
    let sorted = index
        .query_tracks(&TrackQuery {
            artist: Some("Various"),
            album: Some("Collection"),
            ..TrackQuery::default()
        })
        .expect("sorted SQL query")
        .items;
    assert_eq!(
        sorted.iter().map(|row| row.id.get()).collect::<Vec<_>>(),
        vec![first_id, second_id]
    );
    let page = index
        .query_tracks(&TrackQuery {
            artist: Some("Various"),
            album: Some("Collection"),
            limit: 1,
            offset: 1,
            ..TrackQuery::default()
        })
        .expect("second page")
        .items;
    assert_eq!(page[0].id.get(), second_id);
    second.metadata.album_artist = Some("   ".into());
    index.upsert_track(&second, 2).expect("blank album artist");
    assert_eq!(
        index
            .query_tracks(&TrackQuery {
                artist: Some("solo"),
                ..TrackQuery::default()
            })
            .expect("artist fallback")
            .items
            .len(),
        1
    );
    let query = TrackQuery {
        format: Some(crate::domain::AudioFormat::Mp3),
        writable: Some(track.writable),
        artist: Some("VARIOUS"),
        album: Some("collection"),
        genre: Some("rock"),
        title_prefix: Some("first"),
        ..TrackQuery::default()
    };
    let found = index.query_tracks(&query).expect("filtered query").items;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title.as_deref(), Some("First Song"));
    let mut other_writability = query.clone();
    other_writability.writable = Some(!track.writable);
    assert!(
        index
            .query_tracks(&other_writability)
            .expect("writable filter")
            .items
            .is_empty()
    );

    track.metadata.genres = vec!["Ambient".into()];
    index.upsert_track(&track, 2).expect("replace genres");
    assert!(
        index
            .query_tracks(&query)
            .expect("stale genre filter")
            .items
            .is_empty()
    );
    index
        .connection
        .execute("UPDATE tracks SET data = '{bad json'", [])
        .expect("damage raw cache");
    let query = TrackQuery {
        genre: Some("ambient"),
        ..TrackQuery::default()
    };
    assert_eq!(
        index
            .query_tracks(&query)
            .expect("SQL projection")
            .items
            .len(),
        1
    );
    assert!(index.tracks().is_err());
}

#[test]
fn rejects_future_schema_version_without_mutating_database() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("future.sqlite");
    let connection = Connection::open(&path).expect("database");
    connection
        .pragma_update(None, "user_version", 3)
        .expect("version");
    drop(connection);
    assert!(matches!(
        Index::open_at(&path, temp.path()),
        Err(LibraryError::InvalidSchema { version: 3 })
    ));
}

#[test]
fn library_ids_scope_queries_and_generation_cleanup() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first_root = temp.path().join("first");
    let second_root = temp.path().join("second");
    std::fs::create_dir_all(&first_root).expect("first root");
    std::fs::create_dir_all(&second_root).expect("second root");
    let mut first_track = fixture_track(&first_root);
    first_track.metadata.genres = vec!["Rock".into()];
    let second_track = fixture_track(&second_root);
    let database = temp.path().join("shared.sqlite");
    let mut first = Index::open_at(&database, &first_root).expect("first library");
    let mut second = Index::open_at(&database, &second_root).expect("second library");
    let first_id = first.upsert_track(&first_track, 1).expect("first track");
    second.upsert_track(&second_track, 1).expect("second track");
    assert_eq!(
        first
            .query_tracks(&TrackQuery::default())
            .expect("first query")
            .items[0]
            .path,
        first_track.snapshot.path
    );
    assert_eq!(
        second
            .query_tracks(&TrackQuery::default())
            .expect("second query")
            .items[0]
            .path,
        second_track.snapshot.path
    );
    std::fs::remove_file(&first_track.snapshot.path).expect("remove first audio");
    first.scan(&first_root, |_, _| true).expect("first rescan");
    assert!(
        first
            .query_tracks(&TrackQuery::default())
            .expect("first empty")
            .items
            .is_empty()
    );
    let orphaned_genres: i64 = first
        .connection
        .query_row(
            "SELECT COUNT(*) FROM track_genres WHERE track_id = ?1",
            [first_id],
            |row| row.get(0),
        )
        .expect("foreign key cascade");
    assert_eq!(orphaned_genres, 0);
    assert_eq!(
        second
            .query_tracks(&TrackQuery::default())
            .expect("second retained")
            .items
            .len(),
        1
    );
}

#[test]
fn title_prefix_treats_sql_wildcards_as_literal_text() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut first = fixture_track(temp.path());
    first.metadata.title = Some("100% Real".into());
    let mut second = first.clone();
    second.snapshot.path = temp.path().join("second.mp3");
    second.metadata.title = Some("100X Real".into());
    let mut index = Index::open_at(&temp.path().join("index.sqlite"), temp.path()).expect("index");
    index.upsert_track(&first, 1).expect("first");
    index.upsert_track(&second, 1).expect("second");
    let found = index
        .query_tracks(&TrackQuery {
            title_prefix: Some("100%"),
            ..TrackQuery::default()
        })
        .expect("literal prefix")
        .items;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, first.snapshot.path);
}

#[test]
fn query_searches_metadata_and_reports_total_before_pagination() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut track = fixture_track(temp.path());
    track.metadata.title = Some("A 100% Song".into());
    track.metadata.artist = Some("Example".into());
    let mut index = Index::open_at(&temp.path().join("index.sqlite"), temp.path()).expect("index");
    let first_id = index.upsert_track(&track, 1).expect("first");
    let mut second = track.clone();
    second.snapshot.path = temp.path().join("second.mp3");
    second.metadata.title = Some("Another Song".into());
    let second_id = index.upsert_track(&second, 1).expect("second");
    let page = index
        .query_tracks(&TrackQuery {
            search: Some("song"),
            limit: 1,
            offset: 1,
            sort: TrackSort::Title,
            ..TrackQuery::default()
        })
        .expect("page");
    assert_eq!(page.total, 2);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id.get(), second_id);
    let literal = index
        .query_tracks(&TrackQuery {
            search: Some("100%"),
            ..TrackQuery::default()
        })
        .expect("literal search");
    assert_eq!(literal.total, 1);
    assert_eq!(literal.items[0].id.get(), first_id);
}

#[test]
fn query_filters_issues_and_get_track_is_scoped_to_library() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first_root = temp.path().join("first");
    let second_root = temp.path().join("second");
    std::fs::create_dir_all(&first_root).expect("first root");
    std::fs::create_dir_all(&second_root).expect("second root");
    let mut track = fixture_track(&first_root);
    track.metadata.album = Some("Album".into());
    track.metadata.album_artist = None;
    let database = temp.path().join("shared.sqlite");
    let mut first = Index::open_at(&database, &first_root).expect("first index");
    let id = first.upsert_track(&track, 1).expect("first track");
    let second = Index::open_at(&database, &second_root).expect("second index");
    assert_eq!(
        first
            .query_tracks(&TrackQuery {
                issues: Some(true),
                ..TrackQuery::default()
            })
            .expect("issues")
            .total,
        1
    );
    assert_eq!(
        first
            .get_track(TrackId::indexed(id).expect("id"))
            .expect("get")
            .expect("track")
            .id
            .get(),
        id
    );
    assert!(
        second
            .get_track(TrackId::indexed(id).expect("id"))
            .expect("get")
            .is_none()
    );
    track.metadata.album_artist = Some("Artist".into());
    track.metadata.artists.clear();
    track.metadata.artwork_count = 0;
    track.metadata.track = None;
    track.metadata.disc = None;
    track.diagnostics.clear();
    track.writable = true;
    first.upsert_track(&track, 2).expect("update");
    assert_eq!(
        first
            .query_tracks(&TrackQuery {
                issues: Some(true),
                ..TrackQuery::default()
            })
            .expect("issues after update")
            .total,
        0
    );
}

#[test]
fn index_open_failure_is_typed() {
    let temp = tempfile::tempdir().unwrap();
    assert!(matches!(
        Index::open_path(temp.path()),
        Err(LibraryError::Database(_))
    ));
}

#[test]
fn incremental_refresh_upserts_and_removes_only_changed_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first_path = temp.path().join("first.mp3");
    let second_path = temp.path().join("second.mp3");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3"),
        &first_path,
    )
    .expect("first fixture");
    let mut index = Index::open_at(&temp.path().join("index.sqlite"), temp.path()).expect("index");
    let first_id = match index.refresh_path(&first_path).expect("create") {
        TrackChange::Upserted(track) => track.id,
        other => panic!("unexpected change: {other:?}"),
    };
    assert_eq!(index.tracks().expect("tracks").len(), 1);

    std::fs::rename(&first_path, &second_path).expect("rename");
    assert!(matches!(
        index.refresh_path(&first_path).expect("remove old path"),
        TrackChange::Removed(id) if id == first_id
    ));
    assert!(matches!(
        index.refresh_path(&second_path).expect("add new path"),
        TrackChange::Upserted(_)
    ));
    assert_eq!(index.tracks().expect("tracks").len(), 1);
    assert_eq!(
        index.tracks().expect("tracks")[0].snapshot.path,
        second_path
    );
}

#[test]
fn incremental_refresh_rejects_paths_outside_library_and_ignores_temporary_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("library");
    std::fs::create_dir(&root).expect("root");
    let mut index = Index::open_at(&temp.path().join("index.sqlite"), &root).expect("index");
    assert!(
        index
            .refresh_path(&temp.path().join("outside.mp3"))
            .is_err()
    );
    assert!(matches!(
        index
            .refresh_path(&root.join(".music-tui-work.mp3"))
            .expect("ignore temp"),
        TrackChange::Unchanged
    ));
}

#[cfg(unix)]
#[test]
fn incremental_refresh_rejects_symlinked_parent_outside_library() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("library");
    let outside = temp.path().join("outside");
    std::fs::create_dir(&root).expect("root");
    std::fs::create_dir(&outside).expect("outside");
    let target = outside.join("song.mp3");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3"),
        &target,
    )
    .expect("fixture");
    symlink(&outside, root.join("link")).expect("symlink");
    let mut index = Index::open_at(&temp.path().join("index.sqlite"), &root).expect("index");
    assert!(index.refresh_path(&root.join("link/song.mp3")).is_err());
    assert!(index.tracks().expect("tracks").is_empty());
}
#[test]
fn scan_emits_tracks_during_background_load() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3"),
        temp.path().join("one.mp3"),
    )
    .expect("fixture");
    let mut index = Index::open_path(&temp.path().join("index.sqlite")).expect("index");
    let mut arrived = Vec::new();
    index
        .scan_with_updates(
            temp.path(),
            |_, _| true,
            |track| arrived.push(track.snapshot.path),
        )
        .expect("scan");
    assert_eq!(arrived, vec![temp.path().join("one.mp3")]);
}

#[test]
fn repeated_scan_preserves_indexed_track_identity() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3"),
        temp.path().join("one.mp3"),
    )
    .expect("fixture");
    let mut index = Index::open_path(&temp.path().join("index.sqlite")).expect("index");
    let first = index.scan(temp.path(), |_, _| true).expect("initial scan");
    assert_eq!(first.reused, 0);
    let original = index.tracks().expect("indexed tracks")[0].id;
    assert!(original.get() > 0);

    let second = index.scan(temp.path(), |_, _| true).expect("repeat scan");
    assert_eq!(second.reused, 1);
    assert_eq!(index.tracks().expect("reused tracks")[0].id, original);
}

#[test]
fn scan_skips_quarantined_subtree() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3");
    std::fs::copy(&fixture, temp.path().join("live.mp3")).expect("copy live");
    let hidden = temp.path().join(crate::quarantine::FOLDER).join("entry");
    std::fs::create_dir_all(&hidden).expect("mkdir");
    std::fs::copy(&fixture, hidden.join("hidden.mp3")).expect("copy hidden");
    let mut index = Index::open_path(&temp.path().join("index.sqlite")).expect("index");
    let report = index.scan(temp.path(), |_, _| true).expect("scan");
    assert_eq!(report.tracks, 1);
    assert_eq!(index.tracks().expect("tracks").len(), 1);
}
