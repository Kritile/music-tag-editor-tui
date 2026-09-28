use super::*;

impl App {
    pub(super) fn action(&mut self, index: usize, sender: &Sender<Message>) {
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
            11 if !self.busy => self.start_online_search(sender.clone()),
            12 if !self.busy => self.mode = Mode::ExportPath(String::new()),
            13 if !self.busy => self.mode = Mode::RenameTemplate(rename::DEFAULT_TEMPLATE.into()),
            14 if !self.busy => match rename::latest(&self.root) {
                Ok(Some(id)) => self.mode = Mode::ConfirmRenameUndo(id),
                Ok(None) => self.status = "No rename journal found".into(),
                Err(error) => self.status = format!("Rename journal lookup failed: {error:#}"),
            },
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
            online: None,
            online_generation: 0,
            export_plan: None,
            cancel_export: Arc::new(AtomicBool::new(false)),
            rename_plan: None,
            post_scan_notice: None,
            preview_row: 0,
        }
    }

    pub(super) fn start_scan(&mut self, sender: Sender<Message>) {
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

    pub(super) fn start_duplicate_search(&mut self, sender: Sender<Message>) {
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

    pub(super) fn open_quarantine_history(&mut self, sender: Sender<Message>) {
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

    pub(super) fn message(&mut self, message: Message) {
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
                        if let Some(notice) = self.post_scan_notice.take() {
                            self.status.push_str(&format!("; {notice}"));
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
            Message::OnlineCandidates(generation, result)
                if generation == self.online_generation =>
            {
                self.busy = false;
                match result {
                    Ok(candidates) => {
                        let count = candidates.len();
                        if let Some(state) = &mut self.online {
                            state.candidates = candidates;
                            state.candidate_row = 0;
                        }
                        self.status = format!("{count} MusicBrainz release candidates");
                        self.mode = Mode::OnlineCandidates;
                    }
                    Err(error) => self.status = format!("MusicBrainz search failed: {error}"),
                }
            }
            Message::OnlineRelease(generation, result) if generation == self.online_generation => {
                self.busy = false;
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
                    Err(error) => self.status = format!("MusicBrainz lookup failed: {error}"),
                }
            }
            Message::ExportPlanned(result) => {
                self.busy = false;
                match result {
                    Ok(plan) => {
                        self.export_plan = Some(plan);
                        self.preview_row = 0;
                        self.mode = Mode::ExportReview;
                    }
                    Err(error) => self.status = format!("Export preview failed: {error}"),
                }
            }
            Message::ExportProgress(count, path) => {
                self.status = format!("Exporting {count}: {}", path.display())
            }
            Message::Exported(result) => {
                self.busy = false;
                self.status = match result {
                    Ok(count) => format!("Export complete: {count} new verified copies"),
                    Err(error) => format!("Export stopped: {error}"),
                };
            }
            Message::RenamePlanned(result) => {
                self.busy = false;
                match result {
                    Ok(plan) => {
                        self.rename_plan = Some(plan);
                        self.preview_row = 0;
                        self.mode = Mode::RenameReview;
                    }
                    Err(error) => self.status = format!("Rename preview failed: {error}"),
                }
            }
            Message::Renamed(result) => {
                self.busy = false;
                self.status = match result {
                    Ok(id) => format!("Renamed files in batch {id}; rescanning…"),
                    Err(error) => format!("Rename stopped: {error}"),
                };
                self.post_scan_notice = Some(self.status.clone());
            }
            Message::RenameUndone(result) => {
                self.busy = false;
                self.status = match result {
                    Ok(()) => "Rename undone; rescanning…".into(),
                    Err(error) => format!("Rename undo failed: {error}"),
                };
                if self.status.starts_with("Rename undone") {
                    self.post_scan_notice = Some(self.status.clone());
                }
            }
            _ => {}
        }
    }

    pub(super) fn run_check(&mut self, scope: CheckScope) {
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
