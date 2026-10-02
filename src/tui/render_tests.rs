use super::*;
use crate::domain::{AudioFormat, Confidence, Edit, Metadata};
use ratatui::backend::TestBackend;

fn sample_track(id: i64, artist: &str, album: &str) -> Track {
    Track {
        id: TrackId::indexed(id).expect("test track ID"),
        snapshot: Snapshot {
            path: PathBuf::from(format!("/synthetic/song-{id}.mp3")),
            size: 0,
            modified_ns: 0,
            sha256: None,
        },
        format: AudioFormat::Mp3,
        raw: vec![],
        metadata: Metadata {
            title: Some(format!("Song {id}")),
            artist: Some(artist.into()),
            album_artist: Some(artist.into()),
            album: Some(album.into()),
            ..Metadata::default()
        },
        diagnostics: vec![],
        writable: true,
        write_reason: String::new(),
    }
}

fn screen(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal.draw(|frame| render(frame, app)).expect("render");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn batch_editor_renders_mixed_values_without_io_and_tolerates_small_terminal() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.tracks = vec![
        sample_track(1, "Band", "Album"),
        sample_track(2, "Band", "Album"),
    ];
    app.selected.extend(app.tracks.iter().map(|track| track.id));
    app.open_batch_editor();
    let rendered = screen(&app, 75, 13);
    assert!(rendered.contains("Edit selected (2 tracks)"), "{rendered}");
    assert!(rendered.contains("<mixed>"), "{rendered}");
    screen(&app, 12, 5);
    screen(&app, 8, 3);
}

#[test]
fn artists_and_albums_render_from_in_memory_tracks() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.tracks = vec![
        sample_track(1, "Blue Band", "First"),
        sample_track(2, "Red Band", "Second"),
    ];
    app.rebuild();
    let artists = screen(&app, 100, 20);
    assert!(artists.contains("Artists *"), "{artists}");
    assert!(artists.contains("Blue Band"), "{artists}");
    app.group_mode = GroupMode::Albums;
    app.rebuild();
    let albums = screen(&app, 100, 20);
    assert!(albums.contains("Albums *"), "{albums}");
    assert!(albums.contains("First — Blue Band"), "{albums}");
    assert!(albums.contains("Second — Red Band"), "{albums}");
}

#[test]
fn issues_tree_and_results_render_synthetic_issue() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    let track = sample_track(1, "Band", "Album");
    app.issues = vec![Issue {
        rule_id: "missing_album_artist".into(),
        track_id: track.id,
        path: track.snapshot.path.clone(),
        description: "Artist needs review".into(),
        confidence: Confidence::Likely,
        verification: String::new(),
        suggestion: None,
    }];
    app.tracks = vec![track];
    app.group_mode = GroupMode::Issues;
    app.rebuild();
    let tree = screen(&app, 100, 16);
    assert!(tree.contains("Issues *"), "{tree}");
    assert!(tree.contains("Has issues"), "{tree}");
    app.mode = Mode::Results;
    let results = screen(&app, 100, 16);
    assert!(results.contains("Check results: All"), "{results}");
    assert!(results.contains("missing_album_artist"), "{results}");
}

#[test]
fn diff_and_recovery_status_render_without_loading_journals() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.review_lines = vec!["song.mp3".into(), "TITLE: Old -> New".into()];
    app.mode = Mode::DiffReview;
    let diff = screen(&app, 70, 12);
    assert!(diff.contains("Per-file diff"), "{diff}");
    assert!(diff.contains("TITLE: Old -> New"), "{diff}");
    app.mode = Mode::Normal;
    app.status = "Recovery batch 42: Verified".into();
    let recovery = screen(&app, 70, 12);
    assert!(
        recovery.contains("Recovery batch 42: Verified"),
        "{recovery}"
    );
}

#[test]
fn duplicate_groups_and_comparison_render_from_report() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    let members = [
        sample_track(1, "Band", "Album"),
        sample_track(2, "Band", "Album"),
    ]
    .into_iter()
    .map(|track| duplicates::DuplicateMember {
        track_id: track.id,
        path: track.snapshot.path,
        artist: track.metadata.artist,
        title: track.metadata.title,
        album: track.metadata.album,
        disc: None,
        number: None,
        format: track.format,
        size: 12,
        sha256: Some("synthetic-hash".into()),
    })
    .collect();
    app.duplicate_report = Some(duplicates::DuplicateReport {
        groups: vec![duplicates::DuplicateGroup {
            kind: duplicates::MatchKind::Exact,
            members,
        }],
        ..Default::default()
    });
    app.mode = Mode::Duplicates;
    let groups = screen(&app, 90, 16);
    assert!(groups.contains("Duplicate groups (1)"), "{groups}");
    assert!(groups.contains("Exact: 2 files"), "{groups}");
    app.mode = Mode::DuplicateCompare;
    let comparison = screen(&app, 90, 20);
    assert!(comparison.contains("Compare: Exact"), "{comparison}");
    assert!(comparison.contains("MOVE:"), "{comparison}");
    assert!(comparison.contains("KEEP:"), "{comparison}");
}

#[test]
fn musicbrainz_candidates_and_matches_render_in_memory() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    let mut state = OnlineState::new(vec![sample_track(1, "Band", "Album")]);
    state.candidates = vec![online::Candidate {
        id: "release-1".into(),
        title: "Remote Album".into(),
        artist: "Band".into(),
        date: Some("2001".into()),
        country: None,
        tracks: 1,
    }];
    state.release = Some(online::Release {
        title: "Remote Album".into(),
        artist: "Band".into(),
        tracks: vec![online::RemoteTrack {
            disc: 1,
            number: 1,
            title: "Remote Song".into(),
            artist: "Band".into(),
        }],
    });
    state.mapping = vec![Some(0)];
    app.online = Some(state);
    app.mode = Mode::OnlineCandidates;
    let candidates = screen(&app, 90, 16);
    assert!(
        candidates.contains("MusicBrainz releases (1)"),
        "{candidates}"
    );
    assert!(candidates.contains("Remote Album"), "{candidates}");
    app.mode = Mode::OnlineMatches;
    let matches = screen(&app, 90, 16);
    assert!(matches.contains("Remote Song"), "{matches}");
    assert!(matches.contains("Song 1"), "{matches}");
    let state = app.online.as_mut().expect("online state");
    state.proposals = vec![online::Proposal {
        local_id: state.local[0].id,
        edit: Edit {
            field: Field::Title,
            value: "Remote Song".into(),
        },
        before: Some("Song 1".into()),
    }];
    state.checked = vec![true];
    app.mode = Mode::OnlineReview;
    let review = screen(&app, 90, 16);
    assert!(
        review.contains("Review MusicBrainz suggestions (1)"),
        "{review}"
    );
    assert!(review.contains("[x] song-1.mp3"), "{review}");
}

#[test]
fn modal_and_tiny_terminal_render_safely() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.mode = Mode::Palette("check".into(), 0);
    let palette = screen(&app, 70, 14);
    assert!(palette.contains("Command palette: check"), "{palette}");
    assert!(palette.contains("Check issues"), "{palette}");
    app.mode = Mode::Confirm;
    let confirmation = screen(&app, 70, 14);
    assert!(
        confirmation.contains("Apply all staged changes?"),
        "{confirmation}"
    );
    app.mode = Mode::CheckScope(0);
    let scope = screen(&app, 70, 14);
    assert!(scope.contains("Current folder (recursive)"), "{scope}");
    app.rename_plan = Some(rename::RenamePlan {
        moves: vec![rename::Move {
            source: PathBuf::from("/synthetic/old.mp3"),
            destination: PathBuf::from("/synthetic/new.mp3"),
            sha256: String::new(),
        }],
    });
    app.mode = Mode::ConfirmRename;
    let rename = screen(&app, 70, 14);
    assert!(rename.contains("Rename preview: 1 files"), "{rename}");
    assert!(rename.contains("old.mp3"), "{rename}");
    app.export_plan = Some(export::ExportPlan {
        root: PathBuf::from("/synthetic"),
        destination: PathBuf::from("/device"),
        bytes_to_copy: 12,
        entries: vec![export::ExportEntry {
            source: PathBuf::from("/synthetic/song.mp3"),
            destination: PathBuf::from("/device/song.mp3"),
            size: 12,
            sha256: String::new(),
            skip: false,
        }],
    });
    app.mode = Mode::ConfirmExport;
    let export = screen(&app, 70, 14);
    assert!(export.contains("Export preview: 1 files"), "{export}");
    assert!(export.contains("COPY"), "{export}");
    assert!(screen(&app, 11, 4).contains("Enlarge"));
}
