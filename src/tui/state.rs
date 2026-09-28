use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    Tree,
    Tracks,
    Inspector,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tab {
    Normalized,
    Raw,
    Device,
    Diff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GroupMode {
    Artists,
    Albums,
    Folders,
    Issues,
    EchoMini,
}

impl GroupMode {
    pub(super) fn next(self) -> Self {
        match self {
            Self::Artists => Self::Albums,
            Self::Albums => Self::Folders,
            Self::Folders => Self::Issues,
            Self::Issues => Self::EchoMini,
            Self::EchoMini => Self::Artists,
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Artists => "Artists",
            Self::Albums => "Albums",
            Self::Folders => "Folders",
            Self::Issues => "Issues",
            Self::EchoMini => "Echo Mini prediction (Experimental)",
        }
    }
}

pub(super) enum Mode {
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
    OnlineCandidates,
    OnlineMatches,
    OnlineReview,
    ExportPath(String),
    ExportReview,
    ConfirmExport,
    RenameTemplate(String),
    RenameReview,
    ConfirmRename,
    ConfirmRenameUndo(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CheckScope {
    Folder,
    Library,
    Album,
    Selected,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum IssueFilter {
    All,
    Generic,
    Echo,
}

impl IssueFilter {
    pub(super) fn next(self) -> Self {
        match self {
            Self::All => Self::Generic,
            Self::Generic => Self::Echo,
            Self::Echo => Self::All,
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Generic => "Generic",
            Self::Echo => "Echo Mini",
        }
    }
}

pub(super) const ACTIONS: &[(Action, &str)] = &[
    (Action::Refresh, "Refresh library"),
    (Action::Check, "Check issues"),
    (Action::PreviewEchoMini, "Preview Echo Mini"),
    (Action::StageSuggestion, "Stage suggested fix"),
    (Action::Edit, "Edit tags"),
    (Action::ReviewDiff, "Review diffs"),
    (Action::Apply, "Apply staged changes"),
    (Action::Undo, "Undo latest transaction"),
    (Action::InspectRecovery, "Inspect recovery"),
    (Action::FindDuplicates, "Find duplicates"),
    (Action::InspectQuarantine, "Inspect quarantine"),
    (Action::LookupAlbum, "Match album with MusicBrainz"),
    (Action::Export, "Export copy to device"),
    (Action::Rename, "Rename selected tracks or current album"),
    (Action::UndoRename, "Undo latest rename"),
];
pub(super) const SCOPES: &[&str] = &[
    "Current folder (recursive)",
    "Entire library",
    "Current album",
    "Selected tracks",
];

pub(super) fn matching_actions(input: &str) -> Vec<usize> {
    ACTIONS
        .iter()
        .enumerate()
        .filter(|(_, (_, label))| label.to_lowercase().contains(&input.to_lowercase()))
        .map(|(index, _)| index)
        .collect()
}

pub(super) fn wrapped_index(index: usize, length: usize, down: bool) -> usize {
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
pub(super) enum TreeKey {
    Artist(String),
    Album { artist: String, key: String },
    Flat(String),
}

pub(super) fn artist_name(track: &Track) -> &str {
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

pub(super) fn album_name(track: &Track) -> &str {
    track
        .metadata
        .album
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("<unknown album>")
}

pub(super) fn name_order(name: &str) -> (bool, String, String) {
    (name.starts_with('<'), name.to_lowercase(), name.to_string())
}

pub(super) fn track_order(track: &Track) -> ((bool, u32), (bool, u32), String, &Path) {
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

pub(super) struct App {
    pub(super) root: PathBuf,
    pub(super) tracks: Vec<Track>,
    pub(super) tracks_dirty: bool,
    pub(super) issues: Vec<Issue>,
    pub(super) checked: bool,
    pub(super) issue_filter: IssueFilter,
    pub(super) issue_row: usize,
    pub(super) review_lines: Vec<String>,
    pub(super) review_row: usize,
    pub(super) staged: Vec<Pending>,
    pub(super) group_mode: GroupMode,
    pub(super) groups: Vec<String>,
    pub(super) tree_keys: Vec<TreeKey>,
    pub(super) group_index: usize,
    pub(super) cursor_key: Option<TreeKey>,
    pub(super) open_artist: Option<String>,
    pub(super) selected_album: Option<String>,
    pub(super) selected_group: Option<String>,
    pub(super) visible: Vec<usize>,
    pub(super) row: usize,
    pub(super) row_anchor: Option<i64>,
    pub(super) inspector_scroll: u16,
    pub(super) selected: BTreeSet<i64>,
    pub(super) focus: Focus,
    pub(super) tab: Tab,
    pub(super) mode: Mode,
    pub(super) filter: String,
    pub(super) status: String,
    pub(super) generation: u64,
    pub(super) busy: bool,
    pub(super) cancel_scan: Arc<AtomicBool>,
    pub(super) duplicate_report: Option<duplicates::DuplicateReport>,
    pub(super) duplicate_group: usize,
    pub(super) duplicate_member: usize,
    pub(super) quarantine_records: Vec<quarantine::Record>,
    pub(super) quarantine_row: usize,
    pub(super) quarantine_journal: Option<PathBuf>,
    pub(super) cancel_duplicates: Arc<AtomicBool>,
    pub(super) online: Option<OnlineState>,
    pub(super) online_generation: u64,
    pub(super) export_plan: Option<export::ExportPlan>,
    pub(super) cancel_export: Arc<AtomicBool>,
    pub(super) rename_plan: Option<rename::RenamePlan>,
    pub(super) post_scan_notice: Option<String>,
    pub(super) preview_row: usize,
}

pub(super) struct OnlineState {
    pub(super) local: Vec<Track>,
    pub(super) candidates: Vec<online::Candidate>,
    pub(super) candidate_row: usize,
    pub(super) release: Option<online::Release>,
    pub(super) mapping: Vec<Option<usize>>,
    pub(super) local_row: usize,
    pub(super) remote_row: usize,
    pub(super) proposals: Vec<online::Proposal>,
    pub(super) checked: Vec<bool>,
    pub(super) proposal_row: usize,
}

impl OnlineState {
    pub(super) fn new(local: Vec<Track>) -> Self {
        Self {
            local,
            candidates: Vec::new(),
            candidate_row: 0,
            release: None,
            mapping: Vec::new(),
            local_row: 0,
            remote_row: 0,
            proposals: Vec::new(),
            checked: Vec::new(),
            proposal_row: 0,
        }
    }

    pub(super) fn build_proposals(&mut self) {
        self.proposals.clear();
        if let Some(release) = &self.release {
            for (track, remote) in self.local.iter().zip(&self.mapping) {
                if let Some(remote) = remote.and_then(|index| release.tracks.get(index))
                    && track.writable
                {
                    self.proposals
                        .extend(online::proposals(track, remote, release));
                }
            }
        }
        self.checked = vec![false; self.proposals.len()];
        self.proposal_row = 0;
    }
}
