use super::*;

pub(super) enum Message {
    Progress(u64, usize, PathBuf),
    Track(u64, Box<Track>),
    Loaded(u64, Result<ScanLoaded, String>),
    Applied(Result<String, String>),
    DuplicateProgress(u64, usize, PathBuf),
    DuplicatesLoaded(u64, duplicates::DuplicateReport),
    Quarantined(Result<quarantine::Record, String>),
    Restored(Result<(), String>),
    QuarantineLoaded(Result<Vec<quarantine::Record>, String>),
    OnlineCandidates(u64, Result<Vec<online::Candidate>, String>),
    OnlineRelease(u64, Result<online::Release, String>),
    ExportPlanned(Result<export::ExportPlan, String>),
    ExportProgress(usize, PathBuf),
    Exported(Result<usize, String>),
    RenamePlanned(Result<rename::RenamePlan, String>),
    Renamed(Result<String, String>),
    RenameUndone(Result<(), String>),
}

pub(super) struct ScanLoaded {
    pub(super) tracks: Vec<Track>,
    pub(super) errors: Vec<String>,
    pub(super) cancelled: bool,
}
