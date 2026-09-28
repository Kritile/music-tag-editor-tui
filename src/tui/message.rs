use super::*;

pub(super) enum Message {
    Progress(u64, usize, PathBuf),
    Track(u64, Box<Track>),
    Loaded(u64, Result<ScanLoaded, String>),
    Applied(Result<String, changes::ApplyError>),
    DuplicateProgress(u64, usize, PathBuf),
    DuplicatesLoaded(u64, duplicates::DuplicateReport),
    Quarantined(Result<quarantine::Record, String>),
    Restored(Result<(), String>),
    QuarantineLoaded(Result<Vec<quarantine::Record>, String>),
    OnlineCandidates(
        u64,
        Result<Vec<online::Candidate>, online::MusicBrainzError>,
    ),
    OnlineRelease(u64, Result<online::Release, online::MusicBrainzError>),
    ExportPlanned(Result<export::ExportPlan, export::ExportError>),
    ExportProgress(usize, PathBuf),
    Exported(Result<usize, export::ExportError>),
    RenamePlanned(Result<rename::RenamePlan, rename::RenameError>),
    Renamed(Result<String, rename::RenameError>),
    RenameUndone(Result<(), String>),
}

pub(super) struct ScanLoaded {
    pub(super) tracks: Vec<Track>,
    pub(super) errors: Vec<String>,
    pub(super) cancelled: bool,
}
