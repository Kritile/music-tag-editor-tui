use super::*;

impl App {
    pub(super) fn launch_job(
        &mut self,
        kind: JobKind,
        sender: Sender<Message>,
        work: impl FnOnce(job::JobContext) -> Message + Send + 'static,
    ) -> bool {
        match self.jobs.launch(kind, sender, work) {
            Ok(_) => true,
            Err(error) => {
                self.status = format!("Cannot start background job: {error}");
                false
            }
        }
    }

    pub(super) fn run_command(&mut self, command: Action, sender: &Sender<Message>) {
        self.mode = Mode::Normal;
        match command {
            Action::Refresh if !self.jobs.is_busy() => self.start_scan(sender.clone()),
            Action::Check => self.mode = Mode::CheckScope(0),
            Action::PreviewEchoMini => {
                self.group_mode = GroupMode::EchoMini;
                self.selected_group = None;
                self.cursor_key = None;
                self.group_index = 0;
                self.rebuild();
                self.tab = Tab::Device;
                self.focus = Focus::Inspector;
            }
            Action::StageSuggestion => self.stage_suggestion(),
            Action::Edit => self.mode = Mode::Edit(String::new()),
            Action::BatchEdit => self.open_batch_editor(),
            Action::ReviewDiff | Action::Apply => self.open_diff_review(),
            Action::Undo => match changes::latest_batch(&self.root) {
                Ok(Some(id)) => self.mode = Mode::ConfirmUndo(id),
                Ok(None) => self.status = "No transaction to undo".into(),
                Err(error) => self.status = format!("Undo lookup failed: {error:#}"),
            },
            Action::InspectRecovery => match changes::recover_report(&self.root, None) {
                Ok(lines) => {
                    self.status = if lines.is_empty() {
                        "No recovery journals".into()
                    } else {
                        lines.join(" | ")
                    }
                }
                Err(error) => self.status = format!("Recovery inspection failed: {error:#}"),
            },
            Action::FindDuplicates if !self.jobs.is_busy() => {
                self.start_duplicate_search(sender.clone())
            }
            Action::InspectQuarantine if !self.jobs.is_busy() => {
                self.open_quarantine_history(sender.clone())
            }
            Action::LookupAlbum if !self.jobs.is_busy() => self.start_online_search(sender.clone()),
            Action::Export if !self.jobs.is_busy() => self.mode = Mode::ExportPath(String::new()),
            Action::Rename if !self.jobs.is_busy() => {
                self.mode = Mode::RenameTemplate(rename::DEFAULT_TEMPLATE.into())
            }
            Action::UndoRename if !self.jobs.is_busy() => match rename::latest(&self.root) {
                Ok(Some(id)) => self.mode = Mode::ConfirmRenameUndo(id),
                Ok(None) => self.status = "No rename journal found".into(),
                Err(error) => self.status = format!("Rename journal lookup failed: {error:#}"),
            },
            Action::History if !self.jobs.is_busy() => self.start_history(sender.clone()),
            _ => self.status = "Action unavailable".into(),
        }
    }

    pub(super) fn stage_suggestion(&mut self) {
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

    pub(super) fn open_diff_review(&mut self) {
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

    pub(super) fn new(root: &Path) -> Result<Self> {
        let mut app = Self::with_staged(root, changes::load_staged(root)?);
        app.quarantine_journal = Some(quarantine::journal_dir(root)?);
        Ok(app)
    }

    pub(super) fn with_staged(root: &Path, staged: Vec<Pending>) -> Self {
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
            batch_editor: None,
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
            jobs: JobManager::default(),
            duplicate_report: None,
            duplicate_group: 0,
            duplicate_member: 0,
            quarantine_records: vec![],
            quarantine_row: 0,
            quarantine_journal: None,
            online: None,
            export_plan: None,
            rename_plan: None,
            post_scan_notice: None,
            preview_row: 0,
            history_entries: vec![],
            history_row: 0,
            history_detail_scroll: 0,
        }
    }

    pub(super) fn start_history(&mut self, sender: Sender<Message>) {
        let root = self.root.clone();
        self.mode = Mode::History;
        self.status = "Loading operation history…".into();
        self.launch_job(JobKind::History, sender, move |_| {
            let result = history::list(&root).map_err(|error| error.to_string());
            Message::HistoryLoaded(result)
        });
    }

    pub(super) fn start_scan(&mut self, sender: Sender<Message>) {
        let root = self.root.clone();
        if !self.launch_job(JobKind::Scan, sender, move |job| {
            let result = (|| -> Result<_> {
                let mut index = Index::open(&root)?;
                let report = index.scan_with_updates(
                    &root,
                    |count, path| {
                        job.progress(Message::Progress(count, path.to_path_buf()));
                        !job.cancelled()
                    },
                    |track| {
                        job.progress(Message::Track(Box::new(track)));
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
            Message::Loaded(result)
        }) {
            return;
        }
        self.row_anchor = self.current().map(|track| track.id).or(self.row_anchor);
        self.status = "Scanning…".into();
    }

    pub(super) fn start_incremental_refresh(
        &mut self,
        paths: Vec<PathBuf>,
        sender: Sender<Message>,
    ) -> bool {
        let root = self.root.clone();
        self.launch_job(JobKind::IncrementalRefresh, sender, move |_| {
            let result = (|| -> Result<_> {
                let mut index = Index::open(&root)?;
                paths.iter().map(|path| index.refresh_path(path)).collect()
            })()
            .map_err(|error| format!("{error:#}"));
            Message::IncrementalLoaded(result)
        })
    }

    fn prepare_scan(&mut self) {
        self.tracks.clear();
        self.tracks_dirty = false;
        self.issues.clear();
        self.duplicate_report = None;
        self.selected.clear();
        self.checked = false;
        self.rebuild();
    }

    pub(super) fn start_duplicate_search(&mut self, sender: Sender<Message>) {
        let tracks = self.tracks.clone();
        self.status = "Finding duplicates… press c to cancel".into();
        self.launch_job(JobKind::HashDuplicates, sender, move |job| {
            let report = duplicates::find(&tracks, |count, path| {
                job.progress(Message::DuplicateProgress(count, path.to_path_buf()));
                !job.cancelled()
            });
            Message::DuplicatesLoaded(report)
        });
    }

    pub(super) fn open_quarantine_history(&mut self, sender: Sender<Message>) {
        let Some(dir) = self.quarantine_journal.clone() else {
            self.status = "Quarantine journal is unavailable".into();
            return;
        };
        let root = self.root.clone();
        self.status = "Inspecting quarantine…".into();
        self.launch_job(JobKind::History, sender, move |_| {
            let result = quarantine::inspect(&root, &dir).map_err(|error| format!("{error:#}"));
            Message::QuarantineLoaded(result)
        });
    }

    pub(super) fn duplicate_pair(&self) -> Option<(Snapshot, Snapshot)> {
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

    pub(super) fn message(&mut self, message: Message) -> bool {
        if let Message::Job(event) = message {
            return match event {
                JobEvent::Started(id) if self.jobs.is_active(id) => {
                    if self.jobs.active_kind() == Some(JobKind::Scan) {
                        self.prepare_scan();
                    }
                    true
                }
                JobEvent::Update(id, update) if self.jobs.is_active(id) => {
                    self.message(*update);
                    true
                }
                JobEvent::Completed(id, result) if self.jobs.complete(id) => {
                    self.message(*result);
                    true
                }
                JobEvent::Failed(id, error) if self.jobs.complete(id) => {
                    self.status = format!("Background job failed: {error}");
                    true
                }
                JobEvent::Cancelled(id) if self.jobs.complete(id) => {
                    self.status = "Background job cancelled".into();
                    true
                }
                _ => false,
            };
        }
        match message {
            Message::Job(_) => unreachable!(),
            Message::Progress(count, path) => {
                self.status = format!("Scanning {count}: {}", path.display());
            }
            Message::Track(track) => {
                tracing::debug!(track_id = track.id.get(), path = %track.snapshot.path.display(), "track indexed");
                self.tracks.push(*track);
                self.tracks_dirty = true;
            }
            Message::Loaded(result) => match result {
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
                    if let Some(notice) = self.post_scan_notice.take() {
                        self.status.push_str(&format!("; {notice}"));
                    }
                    self.rebuild();
                    self.tracks_dirty = false;
                }
                Err(error) => self.status = error,
            },
            Message::IncrementalLoaded(result) => match result {
                Ok(changes) => {
                    let count = changes
                        .iter()
                        .filter(|change| !matches!(change, crate::library::TrackChange::Unchanged))
                        .count();
                    for change in changes {
                        match change {
                            crate::library::TrackChange::Upserted(track) => {
                                if let Some(previous) =
                                    self.tracks.iter_mut().find(|item| item.id == track.id)
                                {
                                    *previous = *track;
                                } else {
                                    self.tracks.push(*track);
                                }
                            }
                            crate::library::TrackChange::Removed(id) => {
                                self.tracks.retain(|track| track.id != id);
                                self.selected.remove(&id);
                            }
                            crate::library::TrackChange::Unchanged => {}
                        }
                    }
                    self.issues.clear();
                    self.checked = false;
                    self.duplicate_report = None;
                    self.rebuild();
                    self.status = format!("Refreshed {count} changed path(s)");
                }
                Err(error) => {
                    self.status = format!("Incremental refresh failed: {error}; rescanning…")
                }
            },
            Message::Checked(track_count, issues) => self.finish_check(track_count, issues),
            Message::Applied(result) => match result {
                Ok(id) => {
                    self.staged.clear();
                    self.status = format!("Applied batch {id}; backups retained. Rescanning…");
                }
                Err(changes::ApplyError::SourceChanged { path }) => {
                    self.status = format!(
                        "{} changed since preview; rescan before applying",
                        path.display()
                    );
                }
                Err(changes::ApplyError::NothingStaged) => {
                    self.status = "No staged edits to apply".into();
                }
                Err(error) => self.status = error.to_string(),
            },
            Message::HistoryLoaded(result) => match result {
                Ok(entries) => {
                    self.history_entries = entries;
                    self.history_row = self
                        .history_row
                        .min(self.history_entries.len().saturating_sub(1));
                    self.history_detail_scroll = 0;
                    self.status = format!("{} operations in history", self.history_entries.len());
                }
                Err(error) => self.status = format!("History load failed: {error}"),
            },
            Message::HistoryUndone(result) => match result {
                Ok(entries) => {
                    self.history_entries = entries;
                    self.history_row = self
                        .history_row
                        .min(self.history_entries.len().saturating_sub(1));
                    self.history_detail_scroll = 0;
                    self.status = "Operation undone; history updated".into();
                    self.post_scan_notice = Some(self.status.clone());
                }
                Err(error) => self.status = format!("Undo refused: {error}"),
            },
            Message::DuplicateProgress(count, path) => {
                self.status = format!("Checking duplicate {count}: {}", path.display());
            }
            Message::DuplicatesLoaded(report) => {
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
                match result {
                    Ok(record) => self.status = format!("Moved to quarantine: {}", record.id),
                    Err(error) => self.status = format!("Quarantine failed: {error}"),
                }
                self.mode = Mode::Normal;
            }
            Message::Restored(result) => {
                match result {
                    Ok(()) => self.status = "Quarantined file restored".into(),
                    Err(error) => self.status = format!("Restore failed: {error}"),
                }
                self.mode = Mode::Normal;
            }
            Message::QuarantineLoaded(result) => match result {
                Ok(records) => {
                    self.quarantine_records = records;
                    self.quarantine_row = 0;
                    self.mode = Mode::QuarantineHistory;
                }
                Err(error) => self.status = format!("Quarantine inspection failed: {error}"),
            },
            Message::OnlineCandidates(result) => match result {
                Ok(candidates) => {
                    let count = candidates.len();
                    if let Some(state) = &mut self.online {
                        state.candidates = candidates;
                        state.candidate_row = 0;
                    }
                    self.status = format!("{count} MusicBrainz release candidates");
                    self.mode = Mode::OnlineCandidates;
                }
                Err(online::MusicBrainzError::MissingSearchTerms) => {
                    self.status = "Add album and artist tags before searching MusicBrainz".into();
                }
                Err(error) => self.status = format!("MusicBrainz search failed: {error}"),
            },
            Message::OnlineRelease(result) => {
                match result {
                    Ok(release) => {
                        if let Some(state) = &mut self.online {
                            state.mapping = online::match_tracks(&state.local, &release.tracks);
                            state.release = Some(release);
                            state.local_row = 0;
                            state.remote_row = 0;
                            self.mode = Mode::OnlineMatches;
                            self.status = "Review matches; h/l remote track, Enter map, x unmap, r review fields".into();
                        }
                    }
                    Err(online::MusicBrainzError::InvalidReleaseId) => {
                        self.status = "Selected MusicBrainz release ID is invalid".into();
                    }
                    Err(error) => self.status = format!("MusicBrainz lookup failed: {error}"),
                }
            }
            Message::ExportPlanned(result) => match result {
                Ok(plan) => {
                    self.export_plan = Some(plan);
                    self.preview_row = 0;
                    self.mode = Mode::ExportReview;
                }
                Err(export::ExportError::InvalidDestination) => {
                    self.status = "Choose a separate export directory outside the library".into();
                }
                Err(error) => self.status = format!("Export preview failed: {error}"),
            },
            Message::ExportProgress(count, path) => {
                self.status = format!("Exporting {count}: {}", path.display())
            }
            Message::Exported(result) => {
                self.status = match result {
                    Ok(count) => format!("Export complete: {count} new verified copies"),
                    Err(export::ExportError::SourceChanged { path }) => {
                        format!(
                            "{} changed since export preview; review again",
                            path.display()
                        )
                    }
                    Err(export::ExportError::Cancelled { copied }) => {
                        format!("Export cancelled after {copied} verified copies")
                    }
                    Err(error) => format!("Export stopped: {error}"),
                };
            }
            Message::RenamePlanned(result) => match result {
                Ok(plan) => {
                    self.rename_plan = Some(plan);
                    self.preview_row = 0;
                    self.mode = Mode::RenameReview;
                }
                Err(rename::RenameError::DestinationOccupied { path }) => {
                    self.status = format!("Rename destination is occupied: {}", path.display());
                }
                Err(error) => self.status = format!("Rename preview failed: {error}"),
            },
            Message::Renamed(result) => {
                self.status = match result {
                    Ok(id) => format!("Renamed files in batch {id}; rescanning…"),
                    Err(rename::RenameError::PreviewStale { path }) => {
                        format!(
                            "{} changed since rename preview; review again",
                            path.display()
                        )
                    }
                    Err(error) => format!("Rename stopped: {error}"),
                };
                self.post_scan_notice = Some(self.status.clone());
            }
            Message::RenameUndone(result) => {
                self.status = match result {
                    Ok(()) => "Rename undone; rescanning…".into(),
                    Err(error) => format!("Rename undo failed: {error}"),
                };
                if self.status.starts_with("Rename undone") {
                    self.post_scan_notice = Some(self.status.clone());
                }
            }
        }
        true
    }

    pub(super) fn start_check(&mut self, scope: CheckScope, sender: Sender<Message>) {
        let Some(tracks) = self.check_tracks(scope) else {
            return;
        };
        self.mode = Mode::Normal;
        self.status = format!("Checking {} tracks…", tracks.len());
        self.launch_job(JobKind::Check, sender, move |_| {
            let count = tracks.len();
            Message::Checked(count, rules::inspect(&tracks, true))
        });
    }

    #[cfg(test)]
    pub(super) fn run_check(&mut self, scope: CheckScope) {
        if let Some(tracks) = self.check_tracks(scope) {
            let count = tracks.len();
            self.finish_check(count, rules::inspect(&tracks, true));
        }
    }

    fn check_tracks(&mut self, scope: CheckScope) -> Option<Vec<Track>> {
        if self.jobs.is_busy() {
            self.status = "Wait for the library scan to finish before checking".into();
            self.mode = Mode::Normal;
            return None;
        }
        let folder = if scope == CheckScope::Folder {
            match self.folder_for_check() {
                Some(folder) => folder,
                None => {
                    self.status =
                        "Current folder is unavailable; clear search or choose a track".into();
                    self.mode = Mode::Normal;
                    return None;
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
        Some(
            self.tracks
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
                .collect(),
        )
    }

    fn finish_check(&mut self, track_count: usize, issues: Vec<Issue>) {
        self.issues = issues;
        self.checked = true;
        self.issue_row = 0;
        self.issue_filter = IssueFilter::All;
        self.status = format!(
            "Check: {} tracks, {} issues",
            track_count,
            self.issues.len()
        );
        self.mode = Mode::Results;
        self.rebuild();
    }

    pub(super) fn stage_edit(&mut self, edit: Edit) {
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
}
