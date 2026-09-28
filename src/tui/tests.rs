use super::views::render_tree;
use super::*;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::backend::TestBackend;

fn press(app: &mut App, code: KeyCode) -> bool {
    let (sender, _receiver) = mpsc::channel();
    app.key(KeyEvent::new(code, KeyModifiers::NONE), &sender)
}

#[test]
fn semantic_actions_update_state_without_terminal_events() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    let (sender, _receiver) = mpsc::channel();

    assert!(!app.update(Action::Help, &sender));
    assert!(matches!(app.mode, Mode::Help));
    assert!(!app.update(Action::Back, &sender));
    assert!(matches!(app.mode, Mode::Normal));
    assert!(app.update(Action::Quit, &sender));
}

#[test]
fn confirmation_only_accepts_confirm_or_dismiss_actions() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    let (sender, _receiver) = mpsc::channel();
    app.mode = Mode::Confirm;

    assert!(!app.update(Action::MoveDown, &sender));
    assert!(matches!(app.mode, Mode::Confirm));
    assert!(!app.update(Action::Dismiss, &sender));
    assert!(matches!(app.mode, Mode::Normal));
}

#[test]
fn action_menu_and_palette_dispatch_the_same_command() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);

    app.mode = Mode::Actions(1);
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::CheckScope(0)));

    app.mode = Mode::Palette("check".into(), 0);
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::CheckScope(0)));
}

#[test]
fn musicbrainz_review_requires_manual_mapping_and_field_selection() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    let local = browser_track(1, "/synthetic/one.mp3", "Band", "Album", Some(1), Some(1));
    app.tracks = vec![local.clone()];
    app.online = Some(OnlineState::new(vec![local]));
    app.message(Message::OnlineRelease(
        0,
        Ok(online::Release {
            title: "Album".into(),
            artist: "Band".into(),
            tracks: vec![online::RemoteTrack {
                disc: 1,
                number: 1,
                title: "Corrected title".into(),
                artist: "Band".into(),
            }],
        }),
    ));
    assert!(matches!(app.mode, Mode::OnlineMatches));
    assert_eq!(app.online.as_ref().expect("online").mapping, vec![None]);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.online.as_ref().expect("online").mapping, vec![Some(0)]);
    press(&mut app, KeyCode::Char('r'));
    assert!(matches!(app.mode, Mode::OnlineReview));
    let state = app.online.as_ref().expect("online");
    assert!(
        state
            .proposals
            .iter()
            .any(|proposal| proposal.edit.field == Field::Title)
    );
    assert!(state.checked.iter().all(|selected| !selected));
    press(&mut app, KeyCode::Char(' '));
    assert!(app.online.as_ref().expect("online").checked[0]);
}

#[test]
fn rename_failure_remains_visible_after_rescan() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.message(Message::Renamed(Err("batch 42 stopped".into())));
    app.message(Message::Loaded(
        0,
        Ok(ScanLoaded {
            tracks: vec![],
            errors: vec![],
            cancelled: false,
        }),
    ));
    assert!(app.status.contains("batch 42 stopped"), "{}", app.status);
}

#[test]
fn leaving_pending_musicbrainz_lookup_ignores_late_result() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.online = Some(OnlineState::new(vec![]));
    app.mode = Mode::OnlineCandidates;
    app.busy = true;
    press(&mut app, KeyCode::Esc);
    app.message(Message::OnlineRelease(
        0,
        Ok(online::Release {
            title: "Late".into(),
            artist: "Band".into(),
            tracks: vec![],
        }),
    ));
    assert!(matches!(app.mode, Mode::Normal));
    assert!(!app.busy);
}

#[test]
fn quit_waits_for_active_file_operation() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.busy = true;
    app.status = "Renaming files with recovery journal…".into();
    assert!(!press(&mut app, KeyCode::Char('q')));
    assert!(!press(&mut app, KeyCode::Esc));
}

#[test]
fn export_preview_can_show_last_entry_in_short_terminal() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.export_plan = Some(export::ExportPlan {
        root: PathBuf::from("/synthetic"),
        destination: PathBuf::from("/device"),
        bytes_to_copy: 10,
        entries: (0..12)
            .map(|index| export::ExportEntry {
                source: PathBuf::from(format!("/synthetic/song-{index}.mp3")),
                destination: PathBuf::from(format!("/device/song-{index}.mp3")),
                size: 1,
                sha256: String::new(),
                skip: false,
            })
            .collect(),
    });
    app.mode = Mode::ExportReview;
    for _ in 0..11 {
        press(&mut app, KeyCode::Char('j'));
    }
    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|frame| render(frame, &app)).expect("draw");
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("song-11.mp3"), "{text}");
}

#[test]
fn duplicate_comparison_requires_explicit_quarantine_confirmation() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.tracks = vec![
        browser_track(1, "/synthetic/a.mp3", "Band", "Song", None, None),
        browser_track(2, "/synthetic/b.mp3", "Band", "Song", None, None),
    ];
    app.duplicate_report = Some(crate::duplicates::DuplicateReport {
        groups: vec![crate::duplicates::DuplicateGroup {
            kind: crate::duplicates::MatchKind::Probable,
            members: app
                .tracks
                .iter()
                .map(|track| crate::duplicates::DuplicateMember {
                    track_id: track.id,
                    path: track.snapshot.path.clone(),
                    artist: track.metadata.artist.clone(),
                    title: track.metadata.title.clone(),
                    album: track.metadata.album.clone(),
                    disc: None,
                    number: None,
                    format: track.format.clone(),
                    size: 1,
                    sha256: Some(format!("hash{}", track.id)),
                })
                .collect(),
        }],
        ..Default::default()
    });
    app.mode = Mode::Duplicates;
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::DuplicateCompare));
    press(&mut app, KeyCode::Char('x'));
    assert!(matches!(app.mode, Mode::ConfirmQuarantine(_, _)));
    let backend = TestBackend::new(60, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|frame| render(frame, &app)).expect("draw");
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("Compare:"));
    assert!(
        screen.contains("hash2"),
        "selected file hash was clipped: {screen}"
    );
    press(&mut app, KeyCode::Char('n'));
    assert!(matches!(app.mode, Mode::DuplicateCompare));
}

#[test]
fn action_and_palette_start_shared_background_duplicate_search() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = dir.path().join("a.mp3");
    let b = dir.path().join("b.mp3");
    std::fs::write(&a, b"same").expect("write");
    std::fs::write(&b, b"same").expect("write");
    let mut app = App::with_staged(dir.path(), vec![]);
    app.tracks = vec![
        browser_track(1, a.to_str().expect("path"), "Band", "Album", None, None),
        browser_track(2, b.to_str().expect("path"), "Band", "Album", None, None),
    ];
    for track in &mut app.tracks {
        track.snapshot = tags::snapshot(&track.snapshot.path, false).expect("snapshot");
    }
    assert!(app.duplicate_report.is_none());
    assert_eq!(matching_actions("duplicate"), vec![9]);
    let (sender, receiver) = mpsc::channel();
    app.run_command(Action::FindDuplicates, &sender);
    assert!(app.busy);
    loop {
        let message = receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("background result");
        let finished = matches!(message, Message::DuplicatesLoaded(_, _));
        app.message(message);
        if finished {
            break;
        }
    }
    assert!(matches!(app.mode, Mode::Duplicates));
    assert_eq!(
        app.duplicate_report.as_ref().expect("report").groups.len(),
        1
    );
}

#[test]
fn tui_quarantine_and_restore_flow_uses_journal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = dir.path().join("a.mp3");
    let b = dir.path().join("b.mp3");
    std::fs::write(&a, b"same").expect("write");
    std::fs::write(&b, b"same").expect("write");
    let mut app = App::with_staged(dir.path(), vec![]);
    app.quarantine_journal = Some(dir.path().join("journals"));
    app.tracks = vec![
        browser_track(1, a.to_str().expect("path"), "Band", "Album", None, None),
        browser_track(2, b.to_str().expect("path"), "Band", "Album", None, None),
    ];
    for track in &mut app.tracks {
        track.snapshot = tags::snapshot(&track.snapshot.path, false).expect("snapshot");
    }
    app.duplicate_report = Some(duplicates::find(&app.tracks, |_, _| true));
    app.duplicate_member = 1;
    app.mode = Mode::Duplicates;
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('x'));
    let (sender, receiver) = mpsc::channel();
    app.key(
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        &sender,
    );
    let message = receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("move result");
    assert!(matches!(message, Message::Quarantined(Ok(_))));
    app.message(message);
    assert!(!b.exists());
    app.run_command(Action::InspectQuarantine, &sender);
    assert!(app.busy);
    let message = receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("history result");
    app.message(message);
    assert!(matches!(app.mode, Mode::QuarantineHistory));
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::ConfirmRestore(_)));
    app.key(
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        &sender,
    );
    let message = receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("restore result");
    assert!(matches!(message, Message::Restored(Ok(()))));
    app.message(message);
    assert_eq!(std::fs::read(&b).expect("restored"), b"same");
}

fn browser_track(
    id: i64,
    path: &str,
    artist: &str,
    album: &str,
    disc: Option<u32>,
    number: Option<u32>,
) -> Track {
    Track {
        id,
        snapshot: crate::domain::Snapshot {
            path: PathBuf::from(path),
            size: 0,
            modified_ns: 0,
            sha256: None,
        },
        format: "FLAC".into(),
        raw: vec![],
        diagnostics: vec![],
        writable: true,
        write_reason: String::new(),
        metadata: crate::domain::Metadata {
            title: Some(format!("Song {id}")),
            artist: Some(artist.into()),
            album_artist: Some(artist.into()),
            album: Some(album.into()),
            disc: disc.map(|number| crate::domain::NumberPair {
                number,
                total: None,
            }),
            track: number.map(|number| crate::domain::NumberPair {
                number,
                total: None,
            }),
            ..Default::default()
        },
    }
}

#[test]
fn artist_expands_to_distinct_albums_and_album_open_filters_tracks() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.tracks = vec![
        browser_track(
            1,
            "/synthetic/release-a/z.flac",
            "Band",
            "Shared",
            None,
            Some(2),
        ),
        browser_track(
            2,
            "/synthetic/release-a/a.flac",
            "Band",
            "Shared",
            None,
            Some(1),
        ),
        browser_track(
            3,
            "/synthetic/release-b/b.flac",
            "Band",
            "Shared",
            None,
            Some(1),
        ),
    ];
    app.rebuild();
    assert_eq!(app.groups.len(), 1);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.groups.len(), 3);
    assert_eq!(app.visible.len(), 3);
    assert!(app.focus == Focus::Tree);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.visible.len(), 2);
    assert!(app.focus == Focus::Tracks);
    assert_eq!(app.tracks[app.visible[0]].id, 2);
    assert!(app.groups.iter().any(|label| label.contains("release-a")));
    assert!(app.groups.iter().any(|label| label.contains("release-b")));
}

#[test]
fn same_title_and_folder_with_distinct_release_ids_have_distinct_labels() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    let mut first = browser_track(
        1,
        "/synthetic/shared/one.flac",
        "Band",
        "Shared",
        None,
        Some(1),
    );
    first.metadata.release_id = Some("release-one".into());
    let mut second = browser_track(
        2,
        "/synthetic/shared/two.flac",
        "Band",
        "Shared",
        None,
        Some(2),
    );
    second.metadata.release_id = Some("release-two".into());
    app.tracks = vec![first, second];
    app.rebuild();
    press(&mut app, KeyCode::Enter);
    assert_ne!(app.groups[1], app.groups[2]);
    assert!(app.groups[1].contains("release-one") || app.groups[2].contains("release-one"));
    assert!(app.groups[1].contains("release-two") || app.groups[2].contains("release-two"));
}

#[test]
fn global_albums_show_readable_labels_and_search_matches_album_fields() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.tracks = vec![
        browser_track(1, "/synthetic/a/one.flac", "Band", "Blue", None, Some(1)),
        browser_track(2, "/synthetic/b/two.flac", "Band", "Red", None, Some(1)),
    ];
    app.rebuild();
    press(&mut app, KeyCode::Enter);
    app.filter = "Blue".into();
    app.rebuild();
    assert_eq!(
        app.visible
            .iter()
            .map(|&index| app.tracks[index].id)
            .collect::<Vec<_>>(),
        vec![1]
    );
    app.filter = "Band".into();
    app.rebuild();
    assert_eq!(app.visible.len(), 2);
    app.group_mode = GroupMode::Albums;
    app.cursor_key = None;
    app.group_index = 0;
    app.rebuild();
    assert!(app.groups.iter().any(|label| label.contains("Blue — Band")));
    assert!(app.groups.iter().all(|label| !label.contains('|')));
}

#[test]
fn artists_sort_case_insensitively_with_unknown_last() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    let mut unknown = browser_track(3, "/synthetic/u.flac", "Unknown", "Album", None, None);
    unknown.metadata.artist = None;
    unknown.metadata.album_artist = None;
    app.tracks = vec![
        browser_track(1, "/synthetic/b.flac", "beta", "Album", None, None),
        unknown,
        browser_track(2, "/synthetic/a.flac", "Alpha", "Album", None, None),
    ];
    app.rebuild();
    assert!(app.groups[0].contains("Alpha"));
    assert!(app.groups[1].contains("beta"));
    assert!(app.groups[2].contains("<unknown>"));
}

#[test]
fn expanded_album_selection_is_visible_in_short_tree() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.tracks = (0..6)
        .map(|index| {
            browser_track(
                i64::from(index + 1),
                &format!("/synthetic/album-{index}/song.flac"),
                "Band",
                &format!("Album {index}"),
                None,
                Some(1),
            )
        })
        .collect();
    app.rebuild();
    press(&mut app, KeyCode::Enter);
    app.group_index = app.groups.len() - 1;
    let backend = TestBackend::new(28, 5);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| render_tree(frame, frame.area(), &app))
        .expect("draw");
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Album 5"));
}

#[test]
fn search_stays_in_album_and_backspace_clears_before_going_up() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.tracks = vec![
        browser_track(1, "/synthetic/a/one.flac", "Band", "Blue", None, Some(1)),
        browser_track(2, "/synthetic/b/two.flac", "Band", "Red", None, Some(1)),
        browser_track(3, "/synthetic/c/three.flac", "Other", "Blue", None, Some(1)),
    ];
    app.rebuild();
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    app.filter = "Other".into();
    app.rebuild();
    assert!(app.visible.is_empty());
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.visible.len(), 1);
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.visible.len(), 2);
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.groups.len(), 2);
}

#[test]
fn tracks_sort_by_disc_number_then_title_and_keep_selection_after_rebuild() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.tracks = vec![
        browser_track(1, "/synthetic/a/a.flac", "Band", "Album", Some(2), Some(1)),
        browser_track(2, "/synthetic/a/z.flac", "Band", "Album", Some(1), Some(2)),
        browser_track(3, "/synthetic/a/y.flac", "Band", "Album", Some(1), Some(1)),
        browser_track(4, "/synthetic/a/b.flac", "Band", "Album", None, None),
    ];
    app.rebuild();
    assert_eq!(
        app.visible
            .iter()
            .map(|&index| app.tracks[index].id)
            .collect::<Vec<_>>(),
        vec![3, 2, 1, 4]
    );
    app.focus = Focus::Tracks;
    press(&mut app, KeyCode::Down);
    app.tracks.reverse();
    app.rebuild();
    assert_eq!(app.current().map(|track| track.id), Some(2));
}

#[test]
fn refresh_restores_album_cursor_and_track_when_they_return() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    let band = browser_track(
        1,
        "/synthetic/band/one.flac",
        "Band",
        "Album",
        None,
        Some(1),
    );
    let other = browser_track(
        2,
        "/synthetic/other/two.flac",
        "Other",
        "Album",
        None,
        Some(1),
    );
    app.tracks = vec![band.clone(), other.clone()];
    app.rebuild();
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.current().map(|track| track.id), Some(1));
    let desired_cursor = app.cursor_key.clone();
    app.busy = true;
    app.tracks.clear();
    app.rebuild();
    app.tracks.push(other);
    app.rebuild();
    app.tracks.push(band);
    app.busy = false;
    app.rebuild();
    assert_eq!(app.cursor_key, desired_cursor);
    assert_eq!(app.current().map(|track| track.id), Some(1));
}

#[test]
fn album_check_uses_open_album_even_when_search_has_no_matches() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    let mut track = browser_track(
        1,
        "/synthetic/band/one.flac",
        "Band",
        "Album",
        None,
        Some(1),
    );
    track.metadata.album_artist = None;
    app.tracks.push(track);
    app.rebuild();
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    app.filter = "no match".into();
    app.rebuild();
    assert!(app.visible.is_empty());
    app.run_check(CheckScope::Album);
    assert!(
        app.issues
            .iter()
            .any(|issue| issue.rule_id == "missing_album_artist")
    );
}

#[test]
fn folder_check_uses_open_album_folder_when_search_has_no_matches() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.tracks = vec![
        browser_track(
            1,
            "/synthetic/band/one.flac",
            "Band",
            "Album",
            None,
            Some(1),
        ),
        browser_track(
            2,
            "/synthetic/other/two.flac",
            "Other",
            "Album",
            None,
            Some(1),
        ),
    ];
    for track in &mut app.tracks {
        track.metadata.album_artist = None;
    }
    app.rebuild();
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    app.filter = "no match".into();
    app.rebuild();
    assert!(app.visible.is_empty());
    app.run_check(CheckScope::Folder);
    assert!(app.issues.iter().any(|issue| issue.track_id == 1));
    assert!(app.issues.iter().all(|issue| issue.track_id != 2));
}

#[test]
fn returning_to_artists_restores_the_open_album_cursor() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.tracks = vec![
        browser_track(1, "/synthetic/a/one.flac", "Alpha", "Album", None, Some(1)),
        browser_track(2, "/synthetic/b/two.flac", "Band", "Album", None, Some(1)),
    ];
    app.rebuild();
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.current().map(|track| track.id), Some(2));
    let album_key = app.cursor_key.clone();
    press(&mut app, KeyCode::Char('v'));
    while app.group_mode != GroupMode::Artists {
        press(&mut app, KeyCode::Char('v'));
    }
    assert_eq!(app.cursor_key, album_key);
    assert_eq!(app.tree_keys.get(app.group_index), album_key.as_ref());
    assert_eq!(app.current().map(|track| track.id), Some(2));
}

#[test]
fn inspecting_issue_outside_open_artist_reveals_affected_track() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.tracks = vec![
        browser_track(
            1,
            "/synthetic/band/one.flac",
            "Band",
            "Album",
            None,
            Some(1),
        ),
        browser_track(
            2,
            "/synthetic/other/two.flac",
            "Other",
            "Album",
            None,
            Some(1),
        ),
    ];
    app.rebuild();
    press(&mut app, KeyCode::Enter);
    app.issues = vec![Issue {
        rule_id: "example".into(),
        track_id: 2,
        path: PathBuf::from("/synthetic/other/two.flac"),
        description: String::new(),
        confidence: crate::domain::Confidence::Observed,
        verification: String::new(),
        suggestion: None,
    }];
    app.mode = Mode::Results;
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.current().map(|track| track.id), Some(2));
}

#[test]
fn artist_tree_scrolls_selected_artist_into_view() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.groups = (0..8).map(|index| format!("Artist {index}")).collect();
    app.group_index = 7;
    let backend = TestBackend::new(20, 5);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| render_tree(frame, frame.area(), &app))
        .expect("draw");
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        text.contains("Artist 7"),
        "selected artist is outside the visible tree: {text}"
    );
}

#[test]
fn browser_lists_wrap_and_escape_exits_normal_view() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.groups = vec!["A".into(), "B".into(), "C".into()];
    app.visible = vec![0, 1, 2];
    app.focus = Focus::Tree;
    app.group_index = 2;
    press(&mut app, KeyCode::Down);
    assert_eq!(app.group_index, 0);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.group_index, 2);
    app.focus = Focus::Tracks;
    app.row = 2;
    press(&mut app, KeyCode::Down);
    assert_eq!(app.row, 0);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.row, 2);
    assert!(press(&mut app, KeyCode::Esc));
}

#[test]
fn backspace_clears_group_and_search_without_exiting() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.group_mode = GroupMode::Folders;
    app.selected_group = Some("Artist".into());
    app.filter = "song".into();
    assert!(!press(&mut app, KeyCode::Backspace));
    assert!(app.filter.is_empty());
    assert!(!press(&mut app, KeyCode::Backspace));
    assert!(app.selected_group.is_none());
}

#[test]
fn menus_results_and_diff_wrap_both_directions() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.mode = Mode::Actions(ACTIONS.len() - 1);
    press(&mut app, KeyCode::Down);
    assert!(matches!(app.mode, Mode::Actions(0)));
    press(&mut app, KeyCode::Up);
    assert!(matches!(app.mode, Mode::Actions(index) if index == ACTIONS.len() - 1));
    app.mode = Mode::Palette(String::new(), ACTIONS.len() - 1);
    press(&mut app, KeyCode::Down);
    assert!(matches!(app.mode, Mode::Palette(_, 0)));
    press(&mut app, KeyCode::Up);
    assert!(matches!(app.mode, Mode::Palette(_, index) if index == ACTIONS.len() - 1));
    app.mode = Mode::CheckScope(SCOPES.len() - 1);
    press(&mut app, KeyCode::Down);
    assert!(matches!(app.mode, Mode::CheckScope(0)));
    press(&mut app, KeyCode::Up);
    assert!(matches!(app.mode, Mode::CheckScope(index) if index == SCOPES.len() - 1));
    app.review_lines = vec!["one".into(), "two".into()];
    app.mode = Mode::DiffReview;
    app.review_row = 1;
    press(&mut app, KeyCode::Down);
    assert_eq!(app.review_row, 0);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.review_row, 1);
    app.issues = vec![
        Issue {
            rule_id: "first".into(),
            track_id: 1,
            path: PathBuf::from("/synthetic/one.flac"),
            description: String::new(),
            confidence: crate::domain::Confidence::Observed,
            verification: String::new(),
            suggestion: None,
        },
        Issue {
            rule_id: "second".into(),
            track_id: 2,
            path: PathBuf::from("/synthetic/two.flac"),
            description: String::new(),
            confidence: crate::domain::Confidence::Observed,
            verification: String::new(),
            suggestion: None,
        },
    ];
    app.mode = Mode::Results;
    app.issue_row = 1;
    press(&mut app, KeyCode::Down);
    assert_eq!(app.issue_row, 0);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.issue_row, 1);
}

#[test]
fn action_menu_scrolls_to_selection_in_short_terminal() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.mode = Mode::Actions(ACTIONS.len() - 1);
    let backend = TestBackend::new(100, 8);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|frame| render(frame, &app)).expect("draw");
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        text.contains(ACTIONS[ACTIONS.len() - 1].1),
        "selected action is outside the visible menu: {text}"
    );
}

#[test]
fn empty_and_single_item_lists_keep_valid_cursor() {
    assert_eq!(wrapped_index(0, 0, true), 0);
    assert_eq!(wrapped_index(0, 0, false), 0);
    assert_eq!(wrapped_index(0, 1, true), 0);
    assert_eq!(wrapped_index(0, 1, false), 0);
}

#[test]
fn small_render_and_empty_navigation() {
    let mut app = App::with_staged(Path::new("/synthetic/test"), vec![]);
    app.move_cursor(true);
    assert_eq!(app.row, 0);
    app.focus = Focus::Inspector;
    let backend = TestBackend::new(20, 6);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|frame| render(frame, &app)).expect("draw");
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.symbol() == "N")
    );
}

#[test]
fn launch_starts_in_artists_without_running_checker() {
    let app = App::with_staged(Path::new("/synthetic/test"), vec![]);
    assert_eq!(app.group_mode, GroupMode::Artists);
    assert!(app.focus == Focus::Tree);
    assert!(app.issues.is_empty());
    assert!(!app.checked);
}

#[test]
fn check_defaults_to_current_folder_and_filters_categories() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    let mut track = Track {
        id: 1,
        snapshot: crate::domain::Snapshot {
            path: PathBuf::from("/synthetic/a/one.flac"),
            size: 0,
            modified_ns: 0,
            sha256: None,
        },
        format: "FLAC".into(),
        raw: vec![],
        metadata: crate::domain::Metadata::default(),
        diagnostics: vec![],
        writable: true,
        write_reason: String::new(),
    };
    track.metadata.album = Some("Album".into());
    app.tracks.push(track);
    app.rebuild();
    app.run_check(CheckScope::Folder);
    assert!(app.checked);
    assert!(
        app.issues
            .iter()
            .any(|issue| issue.rule_id == "missing_album_artist")
    );
    assert!(
        app.visible_issues()
            .iter()
            .any(|issue| issue.rule_id == "missing_album_artist")
    );
    app.issue_filter = IssueFilter::Echo;
    assert!(
        app.visible_issues()
            .iter()
            .all(|issue| issue.rule_id.starts_with("echo_"))
    );
}

#[test]
fn artists_group_by_album_artist_and_fall_back_to_artist() {
    let app = App::with_staged(Path::new("/synthetic"), vec![]);
    let track = |album_artist: Option<&str>| Track {
        id: 1,
        snapshot: crate::domain::Snapshot {
            path: PathBuf::from("/synthetic/song.flac"),
            size: 0,
            modified_ns: 0,
            sha256: None,
        },
        format: "FLAC".into(),
        raw: vec![],
        metadata: crate::domain::Metadata {
            artist: Some("Track Artist".into()),
            album_artist: album_artist.map(str::to_string),
            ..Default::default()
        },
        diagnostics: vec![],
        writable: true,
        write_reason: String::new(),
    };
    assert_eq!(app.group_for(&track(Some("Album Artist"))), "Album Artist");
    assert_eq!(app.group_for(&track(None)), "Track Artist");
    assert_eq!(app.group_for(&track(Some("  "))), "Track Artist");
    let mut preview = app;
    preview.group_mode = GroupMode::EchoMini;
    assert_eq!(
        preview.group_for(&track(Some("Album Artist"))),
        "Album Artist → <unknown>"
    );
}

#[test]
fn check_scope_uses_shared_rules_and_only_requested_tracks() {
    let make = |id, path: &str| Track {
        id,
        snapshot: crate::domain::Snapshot {
            path: PathBuf::from(path),
            size: 0,
            modified_ns: 0,
            sha256: None,
        },
        format: "FLAC".into(),
        raw: vec![],
        metadata: crate::domain::Metadata {
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            ..Default::default()
        },
        diagnostics: vec![],
        writable: true,
        write_reason: String::new(),
    };
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.tracks = vec![
        make(1, "/synthetic/a/one.flac"),
        make(2, "/synthetic/a/sub/two.flac"),
        make(3, "/synthetic/b/three.flac"),
    ];
    app.rebuild();
    app.run_check(CheckScope::Folder);
    assert!(app.issues.iter().all(|issue| issue.track_id != 3));
    assert!(app.issues.iter().any(|issue| issue.track_id == 2));
    app.run_check(CheckScope::Library);
    assert_eq!(
        serde_json::to_value(&app.issues).expect("issues"),
        serde_json::to_value(rules::inspect(&app.tracks, true)).expect("shared rules")
    );
    app.selected.insert(3);
    app.run_check(CheckScope::Selected);
    assert!(app.issues.iter().all(|issue| issue.track_id == 3));
    app.row = 0;
    app.run_check(CheckScope::Album);
    assert!(app.issues.iter().all(|issue| issue.track_id == 1));
}

#[test]
fn scan_messages_show_tracks_without_running_check() {
    let mut app = App::with_staged(Path::new("/synthetic"), vec![]);
    app.generation = 1;
    let track = Track {
        id: 1,
        snapshot: crate::domain::Snapshot {
            path: PathBuf::from("/synthetic/song.flac"),
            size: 0,
            modified_ns: 0,
            sha256: None,
        },
        format: "FLAC".into(),
        raw: vec![],
        metadata: crate::domain::Metadata {
            album: Some("Album".into()),
            ..Default::default()
        },
        diagnostics: vec![],
        writable: true,
        write_reason: String::new(),
    };
    app.message(Message::Track(1, Box::new(track)));
    assert_eq!(app.tracks.len(), 1);
    assert!(app.issues.is_empty());
    assert!(!app.checked);
    app.message(Message::Loaded(
        1,
        Ok(ScanLoaded {
            tracks: app.tracks.clone(),
            errors: vec![],
            cancelled: false,
        }),
    ));
    assert!(app.issues.is_empty());
    assert!(!app.checked);
}
