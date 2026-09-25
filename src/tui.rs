use crate::changes::{self, Pending};
use crate::domain::{Edit, Field, Issue, RawValue, Snapshot, Track};
use crate::{duplicates, library::Index, quarantine, rules, tags};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Tree,
    Tracks,
    Inspector,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Normalized,
    Raw,
    Device,
    Diff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupMode {
    Artists,
    Albums,
    Folders,
    Issues,
    EchoMini,
}

impl GroupMode {
    fn next(self) -> Self {
        match self {
            Self::Artists => Self::Albums,
            Self::Albums => Self::Folders,
            Self::Folders => Self::Issues,
            Self::Issues => Self::EchoMini,
            Self::EchoMini => Self::Artists,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Artists => "Artists",
            Self::Albums => "Albums",
            Self::Folders => "Folders",
            Self::Issues => "Issues",
            Self::EchoMini => "Echo Mini prediction (Experimental)",
        }
    }
}

enum Mode {
    Normal,
    Search(String),
    Edit(String),
    Confirm,
    ConfirmUndo(String),
    Help,
    Actions(usize),
    Palette(String, usize),
    CheckScope(usize),
    Results,
    DiffReview,
    Duplicates,
    DuplicateCompare,
    ConfirmQuarantine(Snapshot, Snapshot),
    QuarantineHistory,
    ConfirmRestore(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CheckScope {
    Folder,
    Library,
    Album,
    Selected,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IssueFilter {
    All,
    Generic,
    Echo,
}

impl IssueFilter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Generic,
            Self::Generic => Self::Echo,
            Self::Echo => Self::All,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Generic => "Generic",
            Self::Echo => "Echo Mini",
        }
    }
}

const ACTIONS: &[&str] = &[
    "Refresh library",
    "Check issues",
    "Preview Echo Mini",
    "Stage suggested fix",
    "Edit tags",
    "Review diffs",
    "Apply staged changes",
    "Undo latest transaction",
    "Inspect recovery",
    "Find duplicates",
    "Inspect quarantine",
];
const SCOPES: &[&str] = &[
    "Current folder (recursive)",
    "Entire library",
    "Current album",
    "Selected tracks",
];

fn matching_actions(input: &str) -> Vec<usize> {
    ACTIONS
        .iter()
        .enumerate()
        .filter(|(_, label)| label.to_lowercase().contains(&input.to_lowercase()))
        .map(|(index, _)| index)
        .collect()
}

fn wrapped_index(index: usize, length: usize, down: bool) -> usize {
    if length == 0 {
        return 0;
    }
    let index = index.min(length - 1);
    if down {
        if index + 1 == length { 0 } else { index + 1 }
    } else if index == 0 {
        length - 1
    } else {
        index - 1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TreeKey {
    Artist(String),
    Album { artist: String, key: String },
    Flat(String),
}

fn artist_name(track: &Track) -> &str {
    track
        .metadata
        .album_artist
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or(track
            .metadata
            .artist
            .as_deref()
            .filter(|value| !value.trim().is_empty()))
        .unwrap_or("<unknown>")
}

fn album_name(track: &Track) -> &str {
    track
        .metadata
        .album
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("<unknown album>")
}

fn name_order(name: &str) -> (bool, String, String) {
    (name.starts_with('<'), name.to_lowercase(), name.to_string())
}

fn track_order(track: &Track) -> ((bool, u32), (bool, u32), String, &Path) {
    let number = |pair: Option<&crate::domain::NumberPair>| {
        (pair.is_none(), pair.map_or(0, |value| value.number))
    };
    (
        number(track.metadata.disc.as_ref()),
        number(track.metadata.track.as_ref()),
        track.metadata.title.as_deref().unwrap_or("").to_lowercase(),
        &track.snapshot.path,
    )
}

struct App {
    root: PathBuf,
    tracks: Vec<Track>,
    tracks_dirty: bool,
    issues: Vec<Issue>,
    checked: bool,
    issue_filter: IssueFilter,
    issue_row: usize,
    review_lines: Vec<String>,
    review_row: usize,
    staged: Vec<Pending>,
    group_mode: GroupMode,
    groups: Vec<String>,
    tree_keys: Vec<TreeKey>,
    group_index: usize,
    cursor_key: Option<TreeKey>,
    open_artist: Option<String>,
    selected_album: Option<String>,
    selected_group: Option<String>,
    visible: Vec<usize>,
    row: usize,
    row_anchor: Option<i64>,
    inspector_scroll: u16,
    selected: BTreeSet<i64>,
    focus: Focus,
    tab: Tab,
    mode: Mode,
    filter: String,
    status: String,
    generation: u64,
    busy: bool,
    cancel_scan: Arc<AtomicBool>,
    duplicate_report: Option<duplicates::DuplicateReport>,
    duplicate_group: usize,
    duplicate_member: usize,
    quarantine_records: Vec<quarantine::Record>,
    quarantine_row: usize,
    quarantine_journal: Option<PathBuf>,
    cancel_duplicates: Arc<AtomicBool>,
}

enum Message {
    Progress(u64, usize, PathBuf),
    Track(u64, Box<Track>),
    Loaded(u64, Result<ScanLoaded, String>),
    Applied(Result<String, String>),
    DuplicateProgress(u64, usize, PathBuf),
    DuplicatesLoaded(u64, duplicates::DuplicateReport),
    Quarantined(Result<quarantine::Record, String>),
    Restored(Result<(), String>),
    QuarantineLoaded(Result<Vec<quarantine::Record>, String>),
}

struct ScanLoaded {
    tracks: Vec<Track>,
    errors: Vec<String>,
    cancelled: bool,
}

impl App {
    fn action(&mut self, index: usize, sender: &Sender<Message>) {
        self.mode = Mode::Normal;
        match index {
            0 if !self.busy => self.start_scan(sender.clone()),
            1 => self.mode = Mode::CheckScope(0),
            2 => {
                self.group_mode = GroupMode::EchoMini;
                self.selected_group = None;
                self.cursor_key = None;
                self.group_index = 0;
                self.rebuild();
                self.tab = Tab::Device;
                self.focus = Focus::Inspector;
            }
            3 => self.stage_suggestion(),
            4 => self.mode = Mode::Edit(String::new()),
            5 | 6 => self.open_diff_review(),
            7 => match changes::latest_batch(&self.root) {
                Ok(Some(id)) => self.mode = Mode::ConfirmUndo(id),
                Ok(None) => self.status = "No transaction to undo".into(),
                Err(error) => self.status = format!("Undo lookup failed: {error:#}"),
            },
            8 => match changes::recover_report(&self.root, None) {
                Ok(lines) => {
                    self.status = if lines.is_empty() {
                        "No recovery journals".into()
                    } else {
                        lines.join(" | ")
                    }
                }
                Err(error) => self.status = format!("Recovery inspection failed: {error:#}"),
            },
            9 if !self.busy => self.start_duplicate_search(sender.clone()),
            10 if !self.busy => self.open_quarantine_history(sender.clone()),
            _ => self.status = "Action unavailable".into(),
        }
    }

    fn stage_suggestion(&mut self) {
        let selected_issue = if matches!(self.mode, Mode::Results) {
            self.visible_issues().get(self.issue_row).copied()
        } else {
            self.current().and_then(|t| {
                self.issues
                    .iter()
                    .find(|i| i.track_id == t.id && i.suggestion.is_some())
            })
        };
        let suggestion = selected_issue
            .and_then(|issue| issue.suggestion.clone().map(|edit| (issue.track_id, edit)));
        if let Some((id, edit)) = suggestion {
            if let Some(track) = self.tracks.iter().find(|track| track.id == id) {
                match changes::stage(&self.root, track, edit) {
                    Ok(staged) => {
                        self.staged = staged;
                        self.tab = Tab::Diff;
                        self.mode = Mode::Normal;
                        self.status = format!(
                            "Staged {} file(s); review diff before apply",
                            self.staged.len()
                        );
                    }
                    Err(error) => self.status = format!("Stage failed: {error:#}"),
                }
            }
        } else {
            self.status = "No suggested edit for this result".into();
        }
    }

    fn open_diff_review(&mut self) {
        self.review_lines.clear();
        let (files, values, backup_bytes) = changes::summary(&self.staged);
        self.review_lines.push(format!(
            "{files} files, {values} values, at least {backup_bytes} backup bytes"
        ));
        for pending in &self.staged {
            self.review_lines
                .push(pending.expected.path.display().to_string());
            match tags::snapshot(&pending.expected.path, true) {
                Ok(current) if current == pending.expected => {}
                Ok(_) => self
                    .review_lines
                    .push("  CONFLICT: file changed since preview".into()),
                Err(error) => self.review_lines.push(format!("  CONFLICT: {error:#}")),
            }
            self.review_lines.extend(
                changes::diff(pending)
                    .into_iter()
                    .map(|line| format!("  {line}")),
            );
        }
        self.review_row = 0;
        self.mode = Mode::DiffReview;
    }

    fn new(root: &Path) -> Result<Self> {
        let mut app = Self::with_staged(root, changes::load_staged(root)?);
        app.quarantine_journal = Some(quarantine::journal_dir(root)?);
        Ok(app)
    }

    fn with_staged(root: &Path, staged: Vec<Pending>) -> Self {
        Self {
            root: root.to_path_buf(),
            tracks: vec![],
            tracks_dirty: false,
            issues: vec![],
            checked: false,
            issue_filter: IssueFilter::All,
            issue_row: 0,
            review_lines: vec![],
            review_row: 0,
            staged,
            group_mode: GroupMode::Artists,
            groups: vec![],
            tree_keys: vec![],
            group_index: 0,
            cursor_key: None,
            open_artist: None,
            selected_album: None,
            selected_group: None,
            visible: vec![],
            row: 0,
            row_anchor: None,
            inspector_scroll: 0,
            selected: BTreeSet::new(),
            focus: Focus::Tree,
            tab: Tab::Normalized,
            mode: Mode::Normal,
            filter: String::new(),
            status: "Scanning…".into(),
            generation: 0,
            busy: false,
            cancel_scan: Arc::new(AtomicBool::new(false)),
            duplicate_report: None,
            duplicate_group: 0,
            duplicate_member: 0,
            quarantine_records: vec![],
            quarantine_row: 0,
            quarantine_journal: None,
            cancel_duplicates: Arc::new(AtomicBool::new(false)),
        }
    }

    fn start_scan(&mut self, sender: Sender<Message>) {
        self.generation += 1;
        let generation = self.generation;
        let root = self.root.clone();
        self.cancel_scan.store(true, Ordering::Relaxed);
        self.cancel_scan = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&self.cancel_scan);
        self.busy = true;
        self.row_anchor = self.current().map(|track| track.id).or(self.row_anchor);
        self.tracks.clear();
        self.tracks_dirty = false;
        self.issues.clear();
        self.duplicate_report = None;
        self.selected.clear();
        self.checked = false;
        self.rebuild();
        self.status = "Scanning…".into();
        thread::spawn(move || {
            let result = (|| -> Result<_> {
                let mut index = Index::open(&root)?;
                let progress_sender = sender.clone();
                let report = index.scan_with_updates(
                    &root,
                    |count, path| {
                        let _ = progress_sender.send(Message::Progress(
                            generation,
                            count,
                            path.to_path_buf(),
                        ));
                        !cancelled.load(Ordering::Relaxed)
                    },
                    |track| {
                        let _ = sender.send(Message::Track(generation, Box::new(track)));
                    },
                )?;
                let tracks = index.tracks()?;
                Ok(ScanLoaded {
                    tracks,
                    errors: report.errors,
                    cancelled: report.cancelled,
                })
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(Message::Loaded(generation, result));
        });
    }

    fn start_duplicate_search(&mut self, sender: Sender<Message>) {
        self.cancel_duplicates.store(true, Ordering::Relaxed);
        self.cancel_duplicates = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&self.cancel_duplicates);
        let tracks = self.tracks.clone();
        let generation = self.generation;
        self.busy = true;
        self.status = "Finding duplicates… press c to cancel".into();
        thread::spawn(move || {
            let progress_sender = sender.clone();
            let report = duplicates::find(&tracks, |count, path| {
                let _ = progress_sender.send(Message::DuplicateProgress(
                    generation,
                    count,
                    path.to_path_buf(),
                ));
                !cancelled.load(Ordering::Relaxed)
            });
            let _ = sender.send(Message::DuplicatesLoaded(generation, report));
        });
    }

    fn open_quarantine_history(&mut self, sender: Sender<Message>) {
        let Some(dir) = self.quarantine_journal.clone() else {
            self.status = "Quarantine journal is unavailable".into();
            return;
        };
        let root = self.root.clone();
        self.busy = true;
        self.status = "Inspecting quarantine…".into();
        thread::spawn(move || {
            let result = quarantine::inspect(&root, &dir).map_err(|error| format!("{error:#}"));
            let _ = sender.send(Message::QuarantineLoaded(result));
        });
    }

    fn duplicate_pair(&self) -> Option<(Snapshot, Snapshot)> {
        let group = self
            .duplicate_report
            .as_ref()?
            .groups
            .get(self.duplicate_group)?;
        let selected = group.members.get(self.duplicate_member)?;
        let kept = group
            .members
            .get(if self.duplicate_member == 0 { 1 } else { 0 })?;
        let snapshot = |member: &duplicates::DuplicateMember| {
            let mut snapshot = self
                .tracks
                .iter()
                .find(|track| track.id == member.track_id)?
                .snapshot
                .clone();
            snapshot.sha256 = member.sha256.clone();
            snapshot.sha256.as_ref()?;
            Some(snapshot)
        };
        Some((snapshot(selected)?, snapshot(kept)?))
    }

    fn message(&mut self, message: Message) {
        match message {
            Message::Progress(generation, count, path) if generation == self.generation => {
                self.status = format!("Scanning {count}: {}", path.display());
            }
            Message::Track(generation, track) if generation == self.generation => {
                self.tracks.push(*track);
                self.tracks_dirty = true;
            }
            Message::Loaded(generation, result) if generation == self.generation => {
                self.busy = false;
                match result {
                    Ok(loaded) => {
                        self.tracks = loaded.tracks;
                        self.issues.clear();
                        self.checked = false;
                        self.status = format!(
                            "{} tracks loaded; run Check for issues; {} scan errors",
                            self.tracks.len(),
                            loaded.errors.len()
                        );
                        if let Some(error) = loaded.errors.first() {
                            self.status.push_str(&format!("; first: {error}"));
                        }
                        if loaded.cancelled {
                            self.status
                                .push_str("; scan cancelled; existing index entries retained");
                        }
                        self.rebuild();
                        self.tracks_dirty = false;
                    }
                    Err(error) => self.status = error,
                }
            }
            Message::Applied(result) => {
                self.busy = false;
                match result {
                    Ok(id) => {
                        self.staged.clear();
                        self.status = format!("Applied batch {id}; backups retained. Rescanning…");
                    }
                    Err(error) => self.status = error,
                }
            }
            Message::DuplicateProgress(generation, count, path)
                if generation == self.generation =>
            {
                self.status = format!("Checking duplicate {count}: {}", path.display());
            }
            Message::DuplicatesLoaded(generation, report) if generation == self.generation => {
                self.busy = false;
                self.status = if report.cancelled {
                    "Duplicate search cancelled".into()
                } else {
                    format!(
                        "{} duplicate groups; {} file errors",
                        report.groups.len(),
                        report.errors.len()
                    )
                };
                if !report.cancelled {
                    self.duplicate_report = Some(report);
                    self.duplicate_group = 0;
                    self.duplicate_member = 1;
                    self.mode = Mode::Duplicates;
                }
            }
            Message::Quarantined(result) => {
                self.busy = false;
                match result {
                    Ok(record) => self.status = format!("Moved to quarantine: {}", record.id),
                    Err(error) => self.status = format!("Quarantine failed: {error}"),
                }
                self.mode = Mode::Normal;
            }
            Message::Restored(result) => {
                self.busy = false;
                match result {
                    Ok(()) => self.status = "Quarantined file restored".into(),
                    Err(error) => self.status = format!("Restore failed: {error}"),
                }
                self.mode = Mode::Normal;
            }
            Message::QuarantineLoaded(result) => {
                self.busy = false;
                match result {
                    Ok(records) => {
                        self.quarantine_records = records;
                        self.quarantine_row = 0;
                        self.mode = Mode::QuarantineHistory;
                    }
                    Err(error) => self.status = format!("Quarantine inspection failed: {error}"),
                }
            }
            _ => {}
        }
    }

    fn group_for(&self, track: &Track) -> String {
        match self.group_mode {
            GroupMode::Artists => artist_name(track).into(),
            GroupMode::Albums => rules::album_key(track),
            GroupMode::Folders => track
                .snapshot
                .path
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            GroupMode::Issues => {
                if self.issues.iter().any(|i| i.track_id == track.id) {
                    "Has issues".into()
                } else {
                    "No issues".into()
                }
            }
            GroupMode::EchoMini => {
                let artist = track
                    .metadata
                    .album_artist
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .or(track.metadata.artist.as_deref())
                    .unwrap_or("<unknown>");
                let album = track.metadata.album.as_deref().unwrap_or("<unknown>");
                format!("{artist} → {album}")
            }
        }
    }

    fn run_check(&mut self, scope: CheckScope) {
        if self.busy {
            self.status = "Wait for the library scan to finish before checking".into();
            self.mode = Mode::Normal;
            return;
        }
        let folder = if scope == CheckScope::Folder {
            match self.folder_for_check() {
                Some(folder) => folder,
                None => {
                    self.status =
                        "Current folder is unavailable; clear search or choose a track".into();
                    self.mode = Mode::Normal;
                    return;
                }
            }
        } else {
            self.root.clone()
        };
        let album = if self.group_mode == GroupMode::Artists {
            self.selected_album
                .clone()
                .or_else(|| self.current().map(rules::album_key))
        } else {
            self.current().map(rules::album_key)
        };
        let tracks: Vec<_> = self
            .tracks
            .iter()
            .filter(|track| match scope {
                CheckScope::Folder => track.snapshot.path.starts_with(&folder),
                CheckScope::Library => true,
                CheckScope::Album => album
                    .as_ref()
                    .is_some_and(|key| rules::album_key(track) == *key),
                CheckScope::Selected => self.selected.contains(&track.id),
            })
            .cloned()
            .collect();
        self.issues = rules::inspect(&tracks, true);
        self.checked = true;
        self.issue_row = 0;
        self.issue_filter = IssueFilter::All;
        self.status = format!(
            "Check: {} tracks, {} issues",
            tracks.len(),
            self.issues.len()
        );
        self.mode = Mode::Results;
        self.rebuild();
    }

    fn folder_for_check(&self) -> Option<PathBuf> {
        if let Some(folder) = self
            .current()
            .and_then(|track| track.snapshot.path.parent())
        {
            return Some(folder.to_path_buf());
        }
        let contextual_track = match self.group_mode {
            GroupMode::Artists => self.selected_album.as_ref().and_then(|key| {
                self.tracks.iter().find(|track| {
                    self.open_artist.as_deref() == Some(artist_name(track))
                        && rules::album_key(track) == *key
                })
            }),
            GroupMode::Albums => self.selected_group.as_ref().and_then(|key| {
                self.tracks
                    .iter()
                    .find(|track| self.group_for(track) == *key)
            }),
            GroupMode::Folders => {
                return self
                    .selected_group
                    .as_ref()
                    .map(PathBuf::from)
                    .or_else(|| Some(self.root.clone()));
            }
            GroupMode::Issues | GroupMode::EchoMini => None,
        };
        if let Some(track) = contextual_track {
            return track.snapshot.path.parent().map(Path::to_path_buf);
        }
        if self.open_artist.is_some() || self.selected_group.is_some() {
            None
        } else {
            Some(self.root.clone())
        }
    }

    fn visible_issues(&self) -> Vec<&Issue> {
        self.issues
            .iter()
            .filter(|issue| match self.issue_filter {
                IssueFilter::All => true,
                IssueFilter::Generic => !issue.rule_id.starts_with("echo_"),
                IssueFilter::Echo => issue.rule_id.starts_with("echo_"),
            })
            .collect()
    }

    fn rebuild(&mut self) {
        let previous_key = self.cursor_key.clone();
        let mut rows = Vec::<(TreeKey, String)>::new();
        if self.group_mode == GroupMode::Artists {
            let mut artists = BTreeMap::<String, Vec<usize>>::new();
            for (index, track) in self.tracks.iter().enumerate() {
                artists
                    .entry(artist_name(track).to_string())
                    .or_default()
                    .push(index);
            }
            if !self.busy
                && self
                    .open_artist
                    .as_ref()
                    .is_some_and(|artist| !artists.contains_key(artist))
            {
                self.open_artist = None;
                self.selected_album = None;
            }
            let mut names: Vec<_> = artists.keys().cloned().collect();
            names.sort_by_key(|name| name_order(name));
            for artist in names {
                let tracks = &artists[&artist];
                let open = self.open_artist.as_deref() == Some(artist.as_str());
                rows.push((
                    TreeKey::Artist(artist.clone()),
                    format!(
                        "{} {artist} ({})",
                        if open { "▾" } else { "▸" },
                        tracks.len()
                    ),
                ));
                if !open {
                    continue;
                }
                let mut albums = BTreeMap::<String, Vec<usize>>::new();
                for &index in tracks {
                    albums
                        .entry(rules::album_key(&self.tracks[index]))
                        .or_default()
                        .push(index);
                }
                if !self.busy
                    && self
                        .selected_album
                        .as_ref()
                        .is_some_and(|key| !albums.contains_key(key))
                {
                    self.selected_album = None;
                }
                let mut infos: Vec<_> = albums
                    .iter()
                    .map(|(key, indices)| {
                        let track = &self.tracks[indices[0]];
                        let title = album_name(track).to_string();
                        let folder = track
                            .snapshot
                            .path
                            .parent()
                            .unwrap_or(&self.root)
                            .strip_prefix(&self.root)
                            .unwrap_or(track.snapshot.path.parent().unwrap_or(&self.root))
                            .display()
                            .to_string();
                        let disambiguator = match track.metadata.release_id.as_deref() {
                            Some(id) => format!("{folder} [{id}]"),
                            None => folder,
                        };
                        (key.clone(), title, disambiguator, indices.len())
                    })
                    .collect();
                let mut name_counts = BTreeMap::<String, usize>::new();
                for (_, title, _, _) in &infos {
                    *name_counts.entry(title.to_lowercase()).or_default() += 1;
                }
                infos.sort_by(|a, b| {
                    (name_order(&a.1), &a.2, &a.0).cmp(&(name_order(&b.1), &b.2, &b.0))
                });
                for (key, title, folder, count) in infos {
                    let suffix = if name_counts[&title.to_lowercase()] > 1 {
                        format!(" — {folder}")
                    } else {
                        String::new()
                    };
                    let marker = if self.selected_album.as_deref() == Some(key.as_str()) {
                        "● "
                    } else {
                        "  "
                    };
                    rows.push((
                        TreeKey::Album {
                            artist: artist.clone(),
                            key,
                        },
                        format!("{marker}{title}{suffix} ({count})"),
                    ));
                }
            }
        } else {
            let mut groups = BTreeMap::<String, Vec<usize>>::new();
            for (index, track) in self.tracks.iter().enumerate() {
                groups.entry(self.group_for(track)).or_default().push(index);
            }
            if !self.busy
                && self
                    .selected_group
                    .as_ref()
                    .is_some_and(|group| !groups.contains_key(group))
            {
                self.selected_group = None;
            }
            let mut items: Vec<_> = groups.into_iter().collect();
            items.sort_by(|a, b| {
                let label = |indices: &[usize], key: &str| {
                    if self.group_mode == GroupMode::Albums {
                        let track = &self.tracks[indices[0]];
                        format!("{} — {}", album_name(track), artist_name(track))
                    } else {
                        key.to_string()
                    }
                };
                (name_order(&label(&a.1, &a.0)), &a.0).cmp(&(name_order(&label(&b.1, &b.0)), &b.0))
            });
            let mut name_counts = BTreeMap::<String, usize>::new();
            if self.group_mode == GroupMode::Albums {
                for (_, indices) in &items {
                    let track = &self.tracks[indices[0]];
                    *name_counts
                        .entry(
                            format!("{} — {}", album_name(track), artist_name(track))
                                .to_lowercase(),
                        )
                        .or_default() += 1;
                }
            }
            for (key, indices) in items {
                let label = if self.group_mode == GroupMode::Albums {
                    let track = &self.tracks[indices[0]];
                    let base = format!("{} — {}", album_name(track), artist_name(track));
                    if name_counts[&base.to_lowercase()] > 1 {
                        let folder = track
                            .snapshot
                            .path
                            .parent()
                            .unwrap_or(&self.root)
                            .strip_prefix(&self.root)
                            .unwrap_or(track.snapshot.path.parent().unwrap_or(&self.root));
                        match track.metadata.release_id.as_deref() {
                            Some(id) => format!("{base} — {} [{id}]", folder.display()),
                            None => format!("{base} — {}", folder.display()),
                        }
                    } else {
                        base
                    }
                } else {
                    key.clone()
                };
                rows.push((TreeKey::Flat(key), format!("{label} ({})", indices.len())));
            }
        }
        self.tree_keys = rows.iter().map(|(key, _)| key.clone()).collect();
        self.groups = rows.into_iter().map(|(_, label)| label).collect();
        let matched_index = previous_key
            .as_ref()
            .and_then(|key| self.tree_keys.iter().position(|row| row == key))
            .or_else(|| match previous_key.as_ref() {
                Some(TreeKey::Album { artist, .. }) => self
                    .tree_keys
                    .iter()
                    .position(|row| row == &TreeKey::Artist(artist.clone())),
                _ => None,
            });
        self.group_index = matched_index
            .unwrap_or_else(|| self.group_index.min(self.groups.len().saturating_sub(1)));
        if let Some(key) = self.tree_keys.get(self.group_index) {
            if matched_index.is_some() || !self.busy || previous_key.is_none() {
                self.cursor_key = Some(key.clone());
            }
        } else if !self.busy {
            self.cursor_key = None;
        }
        let filter = self.filter.to_lowercase();
        self.visible = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| {
                (if self.group_mode == GroupMode::Artists {
                    self.open_artist
                        .as_ref()
                        .is_none_or(|artist| artist_name(track) == artist)
                        && self
                            .selected_album
                            .as_ref()
                            .is_none_or(|album| rules::album_key(track) == *album)
                } else {
                    self.selected_group
                        .as_ref()
                        .is_none_or(|group| self.group_for(track) == *group)
                }) && (filter.is_empty()
                    || track
                        .snapshot
                        .path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&filter)
                    || track
                        .metadata
                        .title
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&filter)
                    || track
                        .metadata
                        .artist
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&filter)
                    || track
                        .metadata
                        .album_artist
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&filter)
                    || track
                        .metadata
                        .album
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&filter))
            })
            .map(|(index, _)| index)
            .collect();
        self.visible
            .sort_by(|&a, &b| track_order(&self.tracks[a]).cmp(&track_order(&self.tracks[b])));
        self.row = self
            .row_anchor
            .and_then(|id| {
                self.visible
                    .iter()
                    .position(|&index| self.tracks[index].id == id)
            })
            .unwrap_or(0);
        if !self.busy || self.row_anchor.is_none() {
            self.row_anchor = self.current().map(|track| track.id);
        }
    }

    fn current(&self) -> Option<&Track> {
        self.visible.get(self.row).and_then(|&i| self.tracks.get(i))
    }

    fn move_cursor(&mut self, down: bool) {
        if self.focus == Focus::Inspector {
            self.inspector_scroll = if down {
                self.inspector_scroll.saturating_add(1)
            } else {
                self.inspector_scroll.saturating_sub(1)
            };
            return;
        }
        let (cursor, length) = if self.focus == Focus::Tree {
            (&mut self.group_index, self.groups.len())
        } else {
            (&mut self.row, self.visible.len())
        };
        *cursor = wrapped_index(*cursor, length, down);
        if self.focus == Focus::Tree {
            self.cursor_key = self.tree_keys.get(self.group_index).cloned();
        } else {
            self.row_anchor = self.current().map(|track| track.id);
        }
        self.inspector_scroll = 0;
    }

    fn stage_edit(&mut self, edit: Edit) {
        let targets: Vec<_> = if self.selected.is_empty() {
            self.current().into_iter().cloned().collect()
        } else {
            self.tracks
                .iter()
                .filter(|t| self.selected.contains(&t.id))
                .cloned()
                .collect()
        };
        if targets.is_empty() {
            self.status = "No tracks selected".into();
            return;
        }
        for track in targets {
            match changes::stage(&self.root, &track, edit.clone()) {
                Ok(pending) => self.staged = pending,
                Err(err) => {
                    self.status =
                        format!("Stage failed at {}: {err:#}", track.snapshot.path.display());
                    return;
                }
            }
        }
        self.status = format!(
            "Staged {} file(s); press d for diff, a to apply",
            self.staged.len()
        );
        self.tab = Tab::Diff;
    }

    fn key(&mut self, key: KeyEvent, sender: &Sender<Message>) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        match &mut self.mode {
            Mode::Actions(index) => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Up | KeyCode::Char('k') => {
                    *index = wrapped_index(*index, ACTIONS.len(), false)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *index = wrapped_index(*index, ACTIONS.len(), true)
                }
                KeyCode::Enter => {
                    let chosen = *index;
                    self.action(chosen, sender);
                }
                _ => {}
            },
            Mode::Palette(input, index) => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    input.pop();
                    *index = 0;
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    *index = 0;
                }
                KeyCode::Up => *index = wrapped_index(*index, matching_actions(input).len(), false),
                KeyCode::Down => {
                    *index = wrapped_index(*index, matching_actions(input).len(), true)
                }
                KeyCode::Enter => {
                    let matches = matching_actions(input);
                    if let Some(&chosen) = matches.get(*index) {
                        self.action(chosen, sender);
                    }
                }
                _ => {}
            },
            Mode::CheckScope(index) => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Up | KeyCode::Char('k') => {
                    *index = wrapped_index(*index, SCOPES.len(), false)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *index = wrapped_index(*index, SCOPES.len(), true)
                }
                KeyCode::Enter => {
                    let scope = match *index {
                        1 => CheckScope::Library,
                        2 => CheckScope::Album,
                        3 => CheckScope::Selected,
                        _ => CheckScope::Folder,
                    };
                    self.run_check(scope);
                }
                _ => {}
            },
            Mode::Results => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
                KeyCode::Char('f') => {
                    self.issue_filter = self.issue_filter.next();
                    self.issue_row = 0;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.issue_row =
                        wrapped_index(self.issue_row, self.visible_issues().len(), true)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.issue_row =
                        wrapped_index(self.issue_row, self.visible_issues().len(), false)
                }
                KeyCode::Enter => {
                    if let Some(id) = self
                        .visible_issues()
                        .get(self.issue_row)
                        .map(|issue| issue.track_id)
                        && self.tracks.iter().any(|track| track.id == id)
                    {
                        self.selected_group = None;
                        self.open_artist = None;
                        self.selected_album = None;
                        self.filter.clear();
                        self.row_anchor = Some(id);
                        self.rebuild();
                        self.focus = Focus::Inspector;
                        self.tab = Tab::Raw;
                        self.mode = Mode::Normal;
                    }
                }
                KeyCode::Char('s') => self.stage_suggestion(),
                _ => {}
            },
            Mode::DiffReview => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Down | KeyCode::Char('j') => {
                    self.review_row = wrapped_index(self.review_row, self.review_lines.len(), true)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.review_row = wrapped_index(self.review_row, self.review_lines.len(), false)
                }
                KeyCode::Char('a')
                    if !self.staged.is_empty()
                        && !self
                            .review_lines
                            .iter()
                            .any(|line| line.contains("CONFLICT")) =>
                {
                    self.mode = Mode::Confirm
                }
                _ => {}
            },
            Mode::Duplicates => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
                KeyCode::Down | KeyCode::Char('j') => {
                    self.duplicate_group = wrapped_index(
                        self.duplicate_group,
                        self.duplicate_report.as_ref().map_or(0, |r| r.groups.len()),
                        true,
                    );
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.duplicate_group = wrapped_index(
                        self.duplicate_group,
                        self.duplicate_report.as_ref().map_or(0, |r| r.groups.len()),
                        false,
                    );
                }
                KeyCode::Enter
                    if self
                        .duplicate_report
                        .as_ref()
                        .and_then(|r| r.groups.get(self.duplicate_group))
                        .is_some() =>
                {
                    self.duplicate_member = 1;
                    self.mode = Mode::DuplicateCompare;
                }
                _ => {}
            },
            Mode::DuplicateCompare => match key.code {
                KeyCode::Esc => self.mode = Mode::Duplicates,
                KeyCode::Down | KeyCode::Char('j') => {
                    self.duplicate_member = wrapped_index(
                        self.duplicate_member,
                        self.duplicate_report
                            .as_ref()
                            .and_then(|r| r.groups.get(self.duplicate_group))
                            .map_or(0, |g| g.members.len()),
                        true,
                    );
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.duplicate_member = wrapped_index(
                        self.duplicate_member,
                        self.duplicate_report
                            .as_ref()
                            .and_then(|r| r.groups.get(self.duplicate_group))
                            .map_or(0, |g| g.members.len()),
                        false,
                    );
                }
                KeyCode::Char('x') if !self.busy => {
                    if let Some((selected, kept)) = self.duplicate_pair() {
                        self.mode = Mode::ConfirmQuarantine(selected, kept);
                    } else {
                        self.status = "Refresh duplicate results before moving this file".into();
                    }
                }
                _ => {}
            },
            Mode::ConfirmQuarantine(selected, kept) => {
                let selected = selected.clone();
                let kept = kept.clone();
                self.mode = Mode::DuplicateCompare;
                if key.code == KeyCode::Char('y')
                    && !self.busy
                    && let Some(dir) = self.quarantine_journal.clone()
                {
                    let root = self.root.clone();
                    let sender = sender.clone();
                    self.busy = true;
                    self.status = "Moving selected file to quarantine…".into();
                    thread::spawn(move || {
                        let result = quarantine::move_file(&root, &dir, &selected, &kept)
                            .map_err(|error| format!("{error:#}"));
                        let _ = sender.send(Message::Quarantined(result));
                    });
                }
            }
            Mode::QuarantineHistory => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Down | KeyCode::Char('j') => {
                    self.quarantine_row =
                        wrapped_index(self.quarantine_row, self.quarantine_records.len(), true);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.quarantine_row =
                        wrapped_index(self.quarantine_row, self.quarantine_records.len(), false);
                }
                KeyCode::Enter => {
                    if let Some(record) = self.quarantine_records.get(self.quarantine_row) {
                        if matches!(
                            record.status,
                            quarantine::Status::Moved | quarantine::Status::Restoring
                        ) {
                            self.mode = Mode::ConfirmRestore(record.id.clone());
                        } else {
                            self.status = "This entry cannot be restored automatically".into();
                        }
                    }
                }
                _ => {}
            },
            Mode::ConfirmRestore(id) => {
                let id = id.clone();
                self.mode = Mode::QuarantineHistory;
                if key.code == KeyCode::Char('y')
                    && !self.busy
                    && let Some(dir) = self.quarantine_journal.clone()
                {
                    let root = self.root.clone();
                    let sender = sender.clone();
                    self.busy = true;
                    self.status = "Restoring quarantined file…".into();
                    thread::spawn(move || {
                        let result = quarantine::restore(&root, &dir, &id)
                            .map_err(|error| format!("{error:#}"));
                        let _ = sender.send(Message::Restored(result));
                    });
                }
            }
            Mode::Search(input) => match key.code {
                KeyCode::Enter => {
                    self.filter = input.clone();
                    self.mode = Mode::Normal;
                    self.rebuild();
                }
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) => input.push(c),
                _ => {}
            },
            Mode::Edit(input) => match key.code {
                KeyCode::Enter => {
                    let text = input.clone();
                    self.mode = Mode::Normal;
                    if let Some((field, value)) = text.split_once('=') {
                        let field = match field.trim().to_ascii_lowercase().as_str() {
                            "title" => Some(Field::Title),
                            "artist" => Some(Field::Artist),
                            "albumartist" | "album_artist" => Some(Field::AlbumArtist),
                            "album" => Some(Field::Album),
                            "track" => Some(Field::Track),
                            "disc" => Some(Field::Disc),
                            _ => None,
                        };
                        if let Some(field) = field {
                            self.stage_edit(Edit {
                                field,
                                value: value.trim().to_string(),
                            });
                        } else {
                            self.status =
                                "Unknown field; use title/artist/albumartist/album/track/disc"
                                    .into();
                        }
                    } else {
                        self.status = "Use field=value".into();
                    }
                }
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) => input.push(c),
                _ => {}
            },
            Mode::Confirm => {
                self.mode = Mode::Normal;
                if key.code == KeyCode::Char('y') && !self.busy {
                    let root = self.root.clone();
                    let sender = sender.clone();
                    self.busy = true;
                    self.status = "Applying staged edits…".into();
                    thread::spawn(move || {
                        let result = changes::apply(&root).map_err(|e| format!("{e:#}"));
                        let _ = sender.send(Message::Applied(result));
                    });
                }
            }
            Mode::ConfirmUndo(id) => {
                let id = id.clone();
                self.mode = Mode::Normal;
                if key.code == KeyCode::Char('y') {
                    match changes::undo(&self.root, &id) {
                        Ok(()) => {
                            self.start_scan(sender.clone());
                            self.status = format!("Undid batch {id}; rescanning…");
                        }
                        Err(error) => self.status = format!("Undo failed: {error:#}"),
                    }
                }
            }
            Mode::Help => self.mode = Mode::Normal,
            Mode::Normal => match key.code {
                KeyCode::Char('q') => return true,
                KeyCode::Char('?') => self.mode = Mode::Help,
                KeyCode::Char('j') | KeyCode::Down => self.move_cursor(true),
                KeyCode::Char('k') | KeyCode::Up => self.move_cursor(false),
                KeyCode::Tab => {
                    self.focus = match self.focus {
                        Focus::Tree => Focus::Tracks,
                        Focus::Tracks => Focus::Inspector,
                        Focus::Inspector => Focus::Tree,
                    }
                }
                KeyCode::Char('v') => {
                    self.group_mode = self.group_mode.next();
                    self.selected_group = None;
                    self.cursor_key = if self.group_mode == GroupMode::Artists {
                        self.selected_album
                            .as_ref()
                            .zip(self.open_artist.as_ref())
                            .map(|(key, artist)| TreeKey::Album {
                                artist: artist.clone(),
                                key: key.clone(),
                            })
                            .or_else(|| self.open_artist.as_ref().cloned().map(TreeKey::Artist))
                    } else {
                        None
                    };
                    self.group_index = 0;
                    self.row_anchor = None;
                    self.rebuild();
                }
                KeyCode::Enter if self.focus == Focus::Tree => {
                    match self.tree_keys.get(self.group_index).cloned() {
                        Some(TreeKey::Artist(artist)) => {
                            self.open_artist = Some(artist.clone());
                            self.selected_album = None;
                            self.cursor_key = Some(TreeKey::Artist(artist));
                            self.row_anchor = None;
                            self.rebuild();
                        }
                        Some(TreeKey::Album { key, .. }) => {
                            self.selected_album = Some(key);
                            self.focus = Focus::Tracks;
                            self.row_anchor = None;
                            self.rebuild();
                        }
                        Some(TreeKey::Flat(key)) => {
                            self.selected_group = Some(key);
                            self.focus = Focus::Tracks;
                            self.row_anchor = None;
                            self.rebuild();
                        }
                        None => {}
                    }
                }
                KeyCode::Esc => return true,
                KeyCode::Backspace => {
                    if !self.filter.is_empty() {
                        self.filter.clear();
                    } else if self.group_mode == GroupMode::Artists {
                        if self.selected_album.take().is_some() {
                            self.focus = Focus::Tree;
                            self.cursor_key = self.open_artist.clone().map(TreeKey::Artist);
                        } else if let Some(artist) = self.open_artist.take() {
                            self.cursor_key = Some(TreeKey::Artist(artist));
                        }
                    } else {
                        self.selected_group = None;
                    }
                    self.row_anchor = None;
                    self.rebuild();
                }
                KeyCode::Char('/') => self.mode = Mode::Search(self.filter.clone()),
                KeyCode::Char(' ') => {
                    if let Some(id) = self.current().map(|t| t.id)
                        && !self.selected.insert(id)
                    {
                        self.selected.remove(&id);
                    }
                }
                KeyCode::Char('b') => {
                    for &index in &self.visible {
                        self.selected.insert(self.tracks[index].id);
                    }
                    self.status = format!("Selected {} visible tracks", self.visible.len());
                }
                KeyCode::Char('x') => {
                    self.selected.clear();
                    self.status = "Selection cleared".into();
                }
                KeyCode::Char('e') => self.mode = Mode::Edit(String::new()),
                KeyCode::Char('s') => self.stage_suggestion(),
                KeyCode::Char('d') => {
                    self.tab = Tab::Diff;
                    self.inspector_scroll = 0;
                    self.open_diff_review();
                }
                KeyCode::Char('n') => {
                    self.tab = Tab::Normalized;
                    self.inspector_scroll = 0;
                }
                KeyCode::Char('w') => {
                    self.tab = Tab::Raw;
                    self.inspector_scroll = 0;
                }
                KeyCode::Char('p') => {
                    self.tab = Tab::Device;
                    self.inspector_scroll = 0;
                }
                KeyCode::Char('a') if !self.staged.is_empty() => self.open_diff_review(),
                KeyCode::Char('r') if !self.busy => self.start_scan(sender.clone()),
                KeyCode::Char('C') => self.mode = Mode::CheckScope(0),
                KeyCode::Char('m') => self.mode = Mode::Actions(0),
                KeyCode::Char(':') => self.mode = Mode::Palette(String::new(), 0),
                KeyCode::Char('c') if self.busy && self.status.starts_with("Scanning") => {
                    self.cancel_scan.store(true, Ordering::Relaxed);
                    self.status = "Cancelling scan after current file…".into();
                }
                KeyCode::Char('c') if self.busy && self.status.contains("duplicate") => {
                    self.cancel_duplicates.store(true, Ordering::Relaxed);
                    self.status = "Cancelling duplicate search…".into();
                }
                _ => {}
            },
        }
        false
    }
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

pub fn run(root: &Path) -> Result<()> {
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        previous(info);
    }));
    let mut terminal: Terminal<CrosstermBackend<Stdout>> =
        Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let (sender, receiver): (Sender<Message>, Receiver<Message>) = mpsc::channel();
    let mut app = App::new(root)?;
    app.start_scan(sender.clone());
    loop {
        for _ in 0..100 {
            let Ok(message) = receiver.try_recv() else {
                break;
            };
            let applied = matches!(
                message,
                Message::Applied(Ok(_)) | Message::Quarantined(Ok(_)) | Message::Restored(Ok(()))
            );
            app.message(message);
            if applied {
                app.start_scan(sender.clone());
            }
        }
        if app.tracks_dirty {
            app.rebuild();
            app.tracks_dirty = false;
        }
        terminal.draw(|frame| render(frame, &app))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && app.key(key, &sender)
        {
            break;
        }
    }
    Ok(())
}

fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 12 || area.height < 5 {
        frame.render_widget(Paragraph::new("Enlarge terminal"), area);
        return;
    }
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(area);
    if matches!(app.mode, Mode::Results) {
        render_results(frame, vertical[1], app);
        frame.render_widget(
            Paragraph::new(
                "Results: j/k move | f filter | Enter inspect | s stage suggestion | Esc back",
            )
            .block(Block::default().borders(Borders::TOP)),
            vertical[2],
        );
        return;
    }
    if matches!(
        app.mode,
        Mode::Duplicates
            | Mode::DuplicateCompare
            | Mode::ConfirmQuarantine(_, _)
            | Mode::QuarantineHistory
            | Mode::ConfirmRestore(_)
    ) {
        render_duplicate_screen(frame, vertical[1], app);
        let help = match app.mode {
            Mode::Duplicates => "Duplicate groups: j/k move | Enter compare | Esc back",
            Mode::DuplicateCompare => "j/k choose file | x quarantine chosen file | Esc groups",
            Mode::QuarantineHistory => "Quarantine: j/k move | Enter restore | Esc back",
            Mode::ConfirmQuarantine(_, _) => {
                "Move selected file to quarantine? y confirms; any other key cancels"
            }
            Mode::ConfirmRestore(_) => "Restore selected file? y confirms; any other key cancels",
            _ => "",
        };
        frame.render_widget(
            Paragraph::new(help).block(Block::default().borders(Borders::TOP)),
            vertical[2],
        );
        return;
    }
    if matches!(app.mode, Mode::DiffReview) {
        let height = vertical[1].height.saturating_sub(2) as usize;
        let start = app.review_row.saturating_sub(height.saturating_sub(1));
        let lines = app
            .review_lines
            .iter()
            .skip(start)
            .take(height)
            .enumerate()
            .map(|(offset, line)| {
                ListItem::new(line.as_str()).style(if start + offset == app.review_row {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                })
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            List::new(lines).block(
                Block::default()
                    .title("Per-file diff")
                    .borders(Borders::ALL),
            ),
            vertical[1],
        );
        frame.render_widget(
            Paragraph::new("Review each file | j/k scroll | a apply | Esc back")
                .block(Block::default().borders(Borders::TOP)),
            vertical[2],
        );
        return;
    }
    frame.render_widget(
        Paragraph::new(format!(
            "{} | {} | {} selected | {} staged | Actions [m] | Palette [:] | Check [C]",
            app.root.display(),
            app.group_mode.label(),
            app.selected.len(),
            app.staged.len()
        ))
        .block(Block::default().borders(Borders::BOTTOM)),
        vertical[0],
    );
    let narrow = area.width < 90;
    if narrow {
        match app.focus {
            Focus::Tree => render_tree(frame, vertical[1], app),
            Focus::Tracks => render_tracks(frame, vertical[1], app),
            Focus::Inspector => render_inspector(frame, vertical[1], app),
        }
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(22),
                Constraint::Percentage(38),
                Constraint::Percentage(40),
            ])
            .split(vertical[1]);
        render_tree(frame, cols[0], app);
        render_tracks(frame, cols[1], app);
        render_inspector(frame, cols[2], app);
    }
    let status = match &app.mode {
        Mode::Normal => app.status.clone(),
        Mode::Search(s) => format!("Search: {s}"),
        Mode::Edit(s) => format!("Edit field=value: {s}"),
        Mode::Confirm => "Apply all staged changes? y = confirm, any other key = cancel".into(),
        Mode::ConfirmUndo(id) => format!("Undo batch {id} from verified backups? y = confirm, any other key = cancel"),
        Mode::Help => "m actions | : palette | C check | Tab panels | v grouping | j/k wrap | Enter open artist/album | Backspace clear search/up tree | Space/b select | x clear | / search | e edit | s suggest | n/w/p/d tabs | a apply | r rescan | c cancel | Esc/q quit".into(),
        Mode::Actions(index) => format!("Actions (j/k, Enter, Esc): {}", ACTIONS[*index]),
        Mode::Palette(input, index) => format!("Command palette: {input} | {}", matching_actions(input).get(*index).map(|&i| ACTIONS[i]).unwrap_or("no match")),
        Mode::CheckScope(index) => format!("Check scope (default current folder): {} | j/k, Enter", SCOPES[*index]),
        Mode::ConfirmQuarantine(selected, kept) => format!(
            "Move {} to quarantine; keep {}? y confirms, other key cancels",
            selected.path.display(),
            kept.path.display()
        ),
        Mode::ConfirmRestore(id) => format!("Restore quarantine entry {id}? y confirms"),
        Mode::Results | Mode::DiffReview | Mode::Duplicates | Mode::DuplicateCompare | Mode::QuarantineHistory => unreachable!(),
    };
    frame.render_widget(
        Paragraph::new(status).block(Block::default().borders(Borders::TOP)),
        vertical[2],
    );
    let choices: Option<(String, Vec<(String, bool)>)> = match &app.mode {
        Mode::Actions(index) => Some((
            "Actions".into(),
            ACTIONS
                .iter()
                .enumerate()
                .map(|(i, label)| (label.to_string(), i == *index))
                .collect(),
        )),
        Mode::Palette(input, index) => Some((
            format!("Command palette: {input}"),
            matching_actions(input)
                .iter()
                .enumerate()
                .map(|(row, &i)| (ACTIONS[i].to_string(), row == *index))
                .collect(),
        )),
        Mode::CheckScope(index) => Some((
            "Check scope".into(),
            SCOPES
                .iter()
                .enumerate()
                .map(|(i, label)| (label.to_string(), i == *index))
                .collect(),
        )),
        _ => None,
    };
    if let Some((title, choices)) = choices {
        let popup = Rect {
            x: area.x + area.width / 8,
            y: area.y + 2,
            width: area.width.saturating_mul(3) / 4,
            height: (choices.len() as u16 + 2).min(area.height.saturating_sub(3)),
        };
        let height = popup.height.saturating_sub(2) as usize;
        let selected = choices
            .iter()
            .position(|(_, selected)| *selected)
            .unwrap_or(0);
        let start = selected.saturating_sub(height.saturating_sub(1));
        frame.render_widget(ratatui::widgets::Clear, popup);
        frame.render_widget(
            List::new(
                choices
                    .into_iter()
                    .skip(start)
                    .take(height)
                    .map(|(label, selected)| {
                        ListItem::new(label).style(if selected {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .block(Block::default().title(title).borders(Borders::ALL)),
            popup,
        );
    }
}

fn render_results(frame: &mut Frame, area: Rect, app: &App) {
    let issues = app.visible_issues();
    let height = area.height.saturating_sub(2) as usize;
    let start = app.issue_row.saturating_sub(height.saturating_sub(1));
    let items: Vec<_> = issues
        .iter()
        .skip(start)
        .take(height)
        .enumerate()
        .map(|(offset, issue)| {
            let text = format!(
                "{:?} [{}] {}: {}{}",
                issue.confidence,
                issue.rule_id,
                issue.path.display(),
                issue.description,
                if issue.suggestion.is_some() {
                    " [s: stage fix]"
                } else {
                    ""
                }
            );
            ListItem::new(text).style(if start + offset == app.issue_row {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            })
        })
        .collect();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(format!(
                    "Check results: {} ({} visible)",
                    app.issue_filter.label(),
                    issues.len()
                ))
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn render_duplicate_screen(frame: &mut Frame, area: Rect, app: &App) {
    if matches!(
        app.mode,
        Mode::DuplicateCompare | Mode::ConfirmQuarantine(_, _)
    ) {
        let Some(group) = app
            .duplicate_report
            .as_ref()
            .and_then(|r| r.groups.get(app.duplicate_group))
        else {
            frame.render_widget(Paragraph::new("No duplicate group selected"), area);
            return;
        };
        let Some(selected) = group.members.get(app.duplicate_member) else {
            return;
        };
        let Some(kept) = group
            .members
            .get(if app.duplicate_member == 0 { 1 } else { 0 })
        else {
            return;
        };
        let describe = |label: &str, member: &duplicates::DuplicateMember| {
            format!(
                "{label}: {}\n  {} — {}\n  Album: {} | disc {:?} track {:?}\n  {} | {} bytes\n  SHA-256: {}",
                member.path.display(),
                member.artist.as_deref().unwrap_or("?"),
                member.title.as_deref().unwrap_or("?"),
                member.album.as_deref().unwrap_or("?"),
                member.disc,
                member.number,
                member.format,
                member.size,
                member.sha256.as_deref().unwrap_or("unavailable")
            )
        };
        frame.render_widget(
            Paragraph::new(format!(
                "{}\n\n{}",
                describe("MOVE", selected),
                describe("KEEP", kept)
            ))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(format!("Compare: {:?}", group.kind))
                    .borders(Borders::ALL),
            ),
            area,
        );
        return;
    }
    let height = area.height.saturating_sub(2) as usize;
    let (title, lines, selected): (String, Vec<String>, usize) = match app.mode {
        Mode::Duplicates => {
            let groups = app
                .duplicate_report
                .as_ref()
                .map(|r| r.groups.as_slice())
                .unwrap_or(&[]);
            let lines = groups
                .iter()
                .map(|group| {
                    format!(
                        "{:?}: {} files | {} — {}",
                        group.kind,
                        group.members.len(),
                        group.members[0].artist.as_deref().unwrap_or("?"),
                        group.members[0].title.as_deref().unwrap_or("?")
                    )
                })
                .collect();
            (
                format!("Duplicate groups ({})", groups.len()),
                lines,
                app.duplicate_group,
            )
        }
        Mode::QuarantineHistory | Mode::ConfirmRestore(_) => {
            let lines = app
                .quarantine_records
                .iter()
                .map(|record| {
                    format!(
                        "{:?} {} | {} | kept {}",
                        record.status,
                        record.id,
                        record.original.display(),
                        record.peer.display()
                    )
                })
                .collect();
            ("Quarantine history".into(), lines, app.quarantine_row)
        }
        _ => return,
    };
    let start = selected.saturating_sub(height.saturating_sub(1));
    let items = lines
        .into_iter()
        .skip(start)
        .take(height)
        .enumerate()
        .map(|(offset, line)| {
            ListItem::new(line).style(if start + offset == selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            })
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn render_tree(frame: &mut Frame, area: Rect, app: &App) {
    let height = area.height.saturating_sub(2) as usize;
    let start = app.group_index.saturating_sub(height.saturating_sub(1));
    let items: Vec<ListItem> = app
        .groups
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, group)| {
            let style = if index == app.group_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(group.as_str()).style(style)
        })
        .collect();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(format!(
                    "{}{}",
                    app.group_mode.label(),
                    if app.focus == Focus::Tree { " *" } else { "" }
                ))
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn render_tracks(frame: &mut Frame, area: Rect, app: &App) {
    let height = area.height.saturating_sub(2) as usize;
    let start = app.row.saturating_sub(height.saturating_sub(1));
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .skip(start)
        .take(height)
        .enumerate()
        .map(|(offset, &index)| {
            let track = &app.tracks[index];
            let marker = if app.selected.contains(&track.id) {
                "[x]"
            } else {
                "[ ]"
            };
            let issue = if app.issues.iter().any(|i| i.track_id == track.id) {
                "!"
            } else {
                " "
            };
            let label = format!(
                "{marker}{issue} {} — {}",
                track.metadata.artist.as_deref().unwrap_or("?"),
                track.metadata.title.as_deref().unwrap_or_else(|| track
                    .snapshot
                    .path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?"))
            );
            let style = if start + offset == app.row {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(label).style(style)
        })
        .collect();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(format!(
                    "Tracks {}{}",
                    app.visible.len(),
                    if app.focus == Focus::Tracks { " *" } else { "" }
                ))
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn render_inspector(frame: &mut Frame, area: Rect, app: &App) {
    let title = match app.tab {
        Tab::Normalized => "Normalized (n)",
        Tab::Raw => "Raw (w)",
        Tab::Device => "Echo Mini hypothesis (p)",
        Tab::Diff => "Diff (d)",
    };
    let mut lines = Vec::new();
    if let Some(track) = app.current() {
        lines.push(track.snapshot.path.display().to_string());
        match app.tab {
            Tab::Normalized => {
                let m = &track.metadata;
                lines.extend([
                    format!("TITLE: {:?}", m.title),
                    format!("ARTIST: {:?}", m.artist),
                    format!("ARTISTS: {:?}", m.artists),
                    format!("ALBUMARTIST: {:?}", m.album_artist),
                    format!("ALBUM: {:?}", m.album),
                    format!("TRACK: {:?}", m.track),
                    format!("DISC: {:?}", m.disc),
                    format!("DATE: {:?}", m.date),
                    format!("GENRES: {:?}", m.genres),
                    format!("ARTWORK: {}", m.artwork_count),
                    format!("WRITE: {}", track.write_reason),
                ]);
                lines.extend(track.diagnostics.iter().map(|d| format!("CONFLICT: {d}")));
            }
            Tab::Raw => {
                for raw in &track.raw {
                    let value = match &raw.value {
                        RawValue::Text(s) | RawValue::Locator(s) => s.clone(),
                        RawValue::Binary { bytes, sha256 } => {
                            format!("<binary {bytes} bytes sha256 {sha256}>")
                        }
                    };
                    lines.push(format!(
                        "{}:{} #{} {} = {}",
                        raw.container, raw.native_key, raw.order, raw.detail, value
                    ));
                }
            }
            Tab::Device => {
                let artist = track
                    .metadata
                    .album_artist
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .or(track.metadata.artist.as_deref())
                    .unwrap_or("<unknown>");
                let album = track.metadata.album.as_deref().unwrap_or("<unknown>");
                let projected = rules::echo_projection_key(track);
                let count = app
                    .tracks
                    .iter()
                    .filter(|other| rules::echo_projection_key(other) == projected)
                    .count();
                lines.push(format!("Predicted artist: {artist}"));
                lines.push(format!("Predicted album: {album} ({count} tracks)"));
                lines.push("Confidence: Experimental (tag-derived hypothesis)".into());
                lines.push(format!("Model grouping key: {}", projected));
                lines.push(
                    "Hypothesis only: firmware may group differently or cache prior scans.".into(),
                );
                for issue in app.issues.iter().filter(|i| i.track_id == track.id) {
                    lines.push(format!(
                        "[{:?}] {}: {}",
                        issue.confidence, issue.rule_id, issue.description
                    ));
                }
            }
            Tab::Diff => {
                let (files, values, backup_bytes) = changes::summary(&app.staged);
                lines.push(format!("Batch: {files} files, {values} values, 0 deletions; backup ≥ {backup_bytes} bytes"));
                for pending in app
                    .staged
                    .iter()
                    .filter(|p| p.expected.path == track.snapshot.path)
                {
                    lines.extend(changes::diff(pending));
                }
                if lines.len() == 2 {
                    lines.push("No changes staged for this file".into());
                }
            }
        }
    } else {
        lines.push("No track selected".into());
    }
    frame.render_widget(
        Paragraph::new(lines.join("\n"))
            .scroll((app.inspector_scroll, 0))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(format!(
                        "{title}{}",
                        if app.focus == Focus::Inspector {
                            " *"
                        } else {
                            ""
                        }
                    ))
                    .borders(Borders::ALL),
            ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::backend::TestBackend;

    fn press(app: &mut App, code: KeyCode) -> bool {
        let (sender, _receiver) = mpsc::channel();
        app.key(KeyEvent::new(code, KeyModifiers::NONE), &sender)
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
        app.action(9, &sender);
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
        app.action(10, &sender);
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
            text.contains("Inspect recovery"),
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
}
