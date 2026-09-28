use super::*;

pub(super) enum Message {
    Job(super::job::JobEvent),
    Progress(usize, PathBuf),
    Track(Box<Track>),
    Loaded(Result<ScanLoaded, String>),
    Checked(usize, Vec<Issue>),
    Applied(Result<String, changes::ApplyError>),
    DuplicateProgress(usize, PathBuf),
    DuplicatesLoaded(duplicates::DuplicateReport),
    Quarantined(Result<quarantine::Record, String>),
    Restored(Result<(), String>),
    QuarantineLoaded(Result<Vec<quarantine::Record>, String>),
    OnlineCandidates(Result<Vec<online::Candidate>, online::MusicBrainzError>),
    OnlineRelease(Result<online::Release, online::MusicBrainzError>),
    ExportPlanned(Result<export::ExportPlan, export::ExportError>),
    ExportProgress(usize, PathBuf),
    Exported(Result<usize, export::ExportError>),
    RenamePlanned(Result<rename::RenamePlan, rename::RenameError>),
    Renamed(Result<String, rename::RenameError>),
    RenameUndone(Result<(), String>),
    HistoryLoaded(Result<Vec<history::HistoryEntry>, String>),
    HistoryUndone(Result<Vec<history::HistoryEntry>, String>),
}

impl Message {
    pub(super) fn requests_rescan(&self) -> bool {
        match self {
            Self::Job(super::job::JobEvent::Completed(_, result)) => result.requests_rescan(),
            Self::Applied(Ok(_))
            | Self::Quarantined(Ok(_))
            | Self::Restored(Ok(()))
            | Self::Renamed(_)
            | Self::RenameUndone(Ok(()))
            | Self::HistoryUndone(Ok(_)) => true,
            _ => false,
        }
    }
}

pub(super) struct ScanLoaded {
    pub(super) tracks: Vec<Track>,
    pub(super) errors: Vec<String>,
    pub(super) cancelled: bool,
}
