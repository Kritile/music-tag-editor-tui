use super::*;

/// A user intent after terminal keys have been interpreted for the active mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Quit,
    Help,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Open,
    Back,
    Confirm,
    Dismiss,
    DeleteBackward,
    NextFocus,
    CycleGrouping,
    Search,
    SelectCurrent,
    SelectVisible,
    ClearSelection,
    Edit,
    BatchEdit,
    StageSuggestion,
    ReviewDiff,
    ShowNormalized,
    ShowRaw,
    ShowDevice,
    Apply,
    Refresh,
    Check,
    OpenActions,
    OpenCommandPalette,
    CancelWork,
    FilterIssues,
    Quarantine,
    ClearMapping,
    ReviewMatches,
    StageProposals,
    PreviewEchoMini,
    Undo,
    InspectRecovery,
    FindDuplicates,
    InspectQuarantine,
    LookupAlbum,
    Export,
    Rename,
    UndoRename,
    History,
    Input(char),
}

impl App {
    pub(super) fn update(&mut self, action: Action, sender: &Sender<Message>) -> bool {
        if action == Action::Confirm
            && self.jobs.is_busy()
            && matches!(
                self.mode,
                Mode::Confirm
                    | Mode::ConfirmUndo(_)
                    | Mode::ConfirmQuarantine(_, _)
                    | Mode::ConfirmRestore(_)
                    | Mode::ConfirmExport
                    | Mode::ConfirmRename
                    | Mode::ConfirmRenameUndo(_)
                    | Mode::ConfirmHistoryUndo(_)
            )
        {
            self.status = "Wait for the current job before changing files".into();
            return false;
        }
        if matches!(
            self.mode,
            Mode::Confirm
                | Mode::ConfirmUndo(_)
                | Mode::ConfirmQuarantine(_, _)
                | Mode::ConfirmRestore(_)
                | Mode::ConfirmExport
                | Mode::ConfirmRename
                | Mode::ConfirmRenameUndo(_)
                | Mode::ConfirmHistoryUndo(_)
        ) && !matches!(action, Action::Confirm | Action::Dismiss)
        {
            return false;
        }
        match &mut self.mode {
            Mode::Actions(index) => match action {
                Action::Back => self.mode = Mode::Normal,
                Action::MoveUp => *index = wrapped_index(*index, ACTIONS.len(), false),
                Action::MoveDown => *index = wrapped_index(*index, ACTIONS.len(), true),
                Action::Open => {
                    let chosen = *index;
                    self.run_command(ACTIONS[chosen].0, sender);
                }
                _ => {}
            },
            Mode::Palette(input, index) => match action {
                Action::Back => self.mode = Mode::Normal,
                Action::DeleteBackward => {
                    input.pop();
                    *index = 0;
                }
                Action::Input(c) => {
                    input.push(c);
                    *index = 0;
                }
                Action::MoveUp => {
                    *index = wrapped_index(*index, matching_actions(input).len(), false)
                }
                Action::MoveDown => {
                    *index = wrapped_index(*index, matching_actions(input).len(), true)
                }
                Action::Open => {
                    let matches = matching_actions(input);
                    if let Some(&chosen) = matches.get(*index) {
                        self.run_command(ACTIONS[chosen].0, sender);
                    }
                }
                _ => {}
            },
            Mode::CheckScope(index) => match action {
                Action::Back => self.mode = Mode::Normal,
                Action::MoveUp => *index = wrapped_index(*index, SCOPES.len(), false),
                Action::MoveDown => *index = wrapped_index(*index, SCOPES.len(), true),
                Action::Open => {
                    let scope = match *index {
                        1 => CheckScope::Library,
                        2 => CheckScope::Album,
                        3 => CheckScope::Selected,
                        _ => CheckScope::Folder,
                    };
                    self.start_check(scope, sender.clone());
                }
                _ => {}
            },
            Mode::Results => match action {
                Action::Back => self.mode = Mode::Normal,
                Action::FilterIssues => {
                    self.issue_filter = self.issue_filter.next();
                    self.issue_row = 0;
                }
                Action::MoveDown => {
                    self.issue_row =
                        wrapped_index(self.issue_row, self.visible_issues().len(), true)
                }
                Action::MoveUp => {
                    self.issue_row =
                        wrapped_index(self.issue_row, self.visible_issues().len(), false)
                }
                Action::Open => {
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
                Action::StageSuggestion => self.stage_suggestion(),
                _ => {}
            },
            Mode::DiffReview => match action {
                Action::Back => self.mode = Mode::Normal,
                Action::MoveDown => {
                    self.review_row = wrapped_index(self.review_row, self.review_lines.len(), true)
                }
                Action::MoveUp => {
                    self.review_row = wrapped_index(self.review_row, self.review_lines.len(), false)
                }
                Action::Apply
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
            Mode::History => match action {
                Action::Back => self.mode = Mode::Normal,
                Action::MoveUp => {
                    self.history_row =
                        wrapped_index(self.history_row, self.history_entries.len(), false);
                    self.history_detail_scroll = 0;
                }
                Action::MoveDown => {
                    self.history_row =
                        wrapped_index(self.history_row, self.history_entries.len(), true);
                    self.history_detail_scroll = 0;
                }
                Action::MoveLeft => {
                    self.history_detail_scroll = self.history_detail_scroll.saturating_sub(1)
                }
                Action::MoveRight => {
                    self.history_detail_scroll = self.history_detail_scroll.saturating_add(1)
                }
                Action::Refresh if !self.jobs.is_busy() => self.start_history(sender.clone()),
                Action::Open if !self.jobs.is_busy() => {
                    if let Some(entry) = self.history_entries.get(self.history_row) {
                        if entry.undo_safe {
                            self.mode = Mode::ConfirmHistoryUndo(entry.key.clone());
                        } else {
                            self.status = "Undo unavailable: files or backup changed, or operation is incomplete".into();
                        }
                    }
                }
                _ => {}
            },
            Mode::ConfirmHistoryUndo(key) => {
                let key = key.clone();
                self.mode = Mode::History;
                if action == Action::Confirm && !self.jobs.is_busy() {
                    let root = self.root.clone();
                    self.status = "Verifying and undoing operation…".into();
                    self.launch_job(JobKind::Recovery, sender.clone(), move |_| {
                        let result = history::undo(&root, &key)
                            .and_then(|()| history::list(&root))
                            .map_err(|error| error.to_string());
                        Message::HistoryUndone(result)
                    });
                }
            }
            Mode::Duplicates => match action {
                Action::Back => self.mode = Mode::Normal,
                Action::MoveDown => {
                    self.duplicate_group = wrapped_index(
                        self.duplicate_group,
                        self.duplicate_report.as_ref().map_or(0, |r| r.groups.len()),
                        true,
                    );
                }
                Action::MoveUp => {
                    self.duplicate_group = wrapped_index(
                        self.duplicate_group,
                        self.duplicate_report.as_ref().map_or(0, |r| r.groups.len()),
                        false,
                    );
                }
                Action::Open
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
            Mode::DuplicateCompare => match action {
                Action::Back => self.mode = Mode::Duplicates,
                Action::MoveDown => {
                    self.duplicate_member = wrapped_index(
                        self.duplicate_member,
                        self.duplicate_report
                            .as_ref()
                            .and_then(|r| r.groups.get(self.duplicate_group))
                            .map_or(0, |g| g.members.len()),
                        true,
                    );
                }
                Action::MoveUp => {
                    self.duplicate_member = wrapped_index(
                        self.duplicate_member,
                        self.duplicate_report
                            .as_ref()
                            .and_then(|r| r.groups.get(self.duplicate_group))
                            .map_or(0, |g| g.members.len()),
                        false,
                    );
                }
                Action::Quarantine if !self.jobs.is_busy() => {
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
                if action == Action::Confirm
                    && !self.jobs.is_busy()
                    && let Some(dir) = self.quarantine_journal.clone()
                {
                    let root = self.root.clone();
                    self.status = "Moving selected file to quarantine…".into();
                    self.launch_job(JobKind::Quarantine, sender.clone(), move |_| {
                        let result = quarantine::move_file(&root, &dir, &selected, &kept)
                            .map_err(|error| format!("{error:#}"));
                        Message::Quarantined(result)
                    });
                }
            }
            Mode::QuarantineHistory => match action {
                Action::Back => self.mode = Mode::Normal,
                Action::MoveDown => {
                    self.quarantine_row =
                        wrapped_index(self.quarantine_row, self.quarantine_records.len(), true);
                }
                Action::MoveUp => {
                    self.quarantine_row =
                        wrapped_index(self.quarantine_row, self.quarantine_records.len(), false);
                }
                Action::Open => {
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
                if action == Action::Confirm
                    && !self.jobs.is_busy()
                    && let Some(dir) = self.quarantine_journal.clone()
                {
                    let root = self.root.clone();
                    self.status = "Restoring quarantined file…".into();
                    self.launch_job(JobKind::Recovery, sender.clone(), move |_| {
                        let result = quarantine::restore(&root, &dir, &id)
                            .map_err(|error| format!("{error:#}"));
                        Message::Restored(result)
                    });
                }
            }
            Mode::OnlineCandidates => match action {
                Action::Back => {
                    if self.jobs.is_busy() {
                        self.jobs.cancel(true);
                    }
                    self.mode = Mode::Normal;
                }
                Action::MoveUp => {
                    if let Some(state) = &mut self.online {
                        state.candidate_row =
                            wrapped_index(state.candidate_row, state.candidates.len(), false);
                    }
                }
                Action::MoveDown => {
                    if let Some(state) = &mut self.online {
                        state.candidate_row =
                            wrapped_index(state.candidate_row, state.candidates.len(), true);
                    }
                }
                Action::Open if !self.jobs.is_busy() => self.load_online_release(sender.clone()),
                _ => {}
            },
            Mode::OnlineMatches => match action {
                Action::Back => self.mode = Mode::OnlineCandidates,
                Action::MoveUp => {
                    if let Some(state) = &mut self.online {
                        state.local_row = wrapped_index(state.local_row, state.local.len(), false);
                    }
                }
                Action::MoveDown => {
                    if let Some(state) = &mut self.online {
                        state.local_row = wrapped_index(state.local_row, state.local.len(), true);
                    }
                }
                Action::MoveLeft => {
                    if let Some(state) = &mut self.online {
                        let len = state.release.as_ref().map_or(0, |r| r.tracks.len());
                        state.remote_row = wrapped_index(state.remote_row, len, false);
                    }
                }
                Action::MoveRight => {
                    if let Some(state) = &mut self.online {
                        let len = state.release.as_ref().map_or(0, |r| r.tracks.len());
                        state.remote_row = wrapped_index(state.remote_row, len, true);
                    }
                }
                Action::Open => {
                    if let Some(state) = &mut self.online
                        && state
                            .release
                            .as_ref()
                            .is_some_and(|release| state.remote_row < release.tracks.len())
                    {
                        for mapped in &mut state.mapping {
                            if *mapped == Some(state.remote_row) {
                                *mapped = None;
                            }
                        }
                        if let Some(mapped) = state.mapping.get_mut(state.local_row) {
                            *mapped = Some(state.remote_row);
                        }
                    }
                }
                Action::ClearMapping => {
                    if let Some(state) = &mut self.online
                        && let Some(mapped) = state.mapping.get_mut(state.local_row)
                    {
                        *mapped = None;
                    }
                }
                Action::ReviewMatches => {
                    if let Some(state) = &mut self.online {
                        state.build_proposals();
                        self.mode = Mode::OnlineReview;
                    }
                }
                _ => {}
            },
            Mode::OnlineReview => match action {
                Action::Back => self.mode = Mode::OnlineMatches,
                Action::MoveUp => {
                    if let Some(state) = &mut self.online {
                        state.proposal_row =
                            wrapped_index(state.proposal_row, state.proposals.len(), false);
                    }
                }
                Action::MoveDown => {
                    if let Some(state) = &mut self.online {
                        state.proposal_row =
                            wrapped_index(state.proposal_row, state.proposals.len(), true);
                    }
                }
                Action::SelectCurrent => {
                    if let Some(state) = &mut self.online
                        && let Some(checked) = state.checked.get_mut(state.proposal_row)
                    {
                        *checked = !*checked;
                    }
                }
                Action::StageProposals => self.stage_online_proposals(),
                _ => {}
            },
            Mode::ExportPath(input) => match action {
                Action::Back => self.mode = Mode::Normal,
                Action::DeleteBackward => {
                    input.pop();
                }
                Action::Input(ch) => input.push(ch),
                Action::Open => {
                    let destination = input.clone();
                    self.start_export_preview(destination, sender.clone());
                }
                _ => {}
            },
            Mode::ExportReview => match action {
                Action::Back => self.mode = Mode::Normal,
                Action::Open => self.mode = Mode::ConfirmExport,
                Action::MoveUp => {
                    self.preview_row = wrapped_index(
                        self.preview_row,
                        self.export_plan
                            .as_ref()
                            .map_or(0, |plan| plan.entries.len()),
                        false,
                    );
                }
                Action::MoveDown => {
                    self.preview_row = wrapped_index(
                        self.preview_row,
                        self.export_plan
                            .as_ref()
                            .map_or(0, |plan| plan.entries.len()),
                        true,
                    );
                }
                _ => {}
            },
            Mode::ConfirmExport => {
                if action == Action::Confirm {
                    self.start_export(sender.clone());
                } else {
                    self.mode = Mode::ExportReview;
                }
            }
            Mode::RenameTemplate(input) => match action {
                Action::Back => self.mode = Mode::Normal,
                Action::DeleteBackward => {
                    input.pop();
                }
                Action::Input(ch) => input.push(ch),
                Action::Open => {
                    let template = input.clone();
                    self.start_rename_preview(template, sender.clone());
                }
                _ => {}
            },
            Mode::RenameReview => match action {
                Action::Back => self.mode = Mode::Normal,
                Action::Open => self.mode = Mode::ConfirmRename,
                Action::MoveUp => {
                    self.preview_row = wrapped_index(
                        self.preview_row,
                        self.rename_plan.as_ref().map_or(0, |plan| plan.moves.len()),
                        false,
                    );
                }
                Action::MoveDown => {
                    self.preview_row = wrapped_index(
                        self.preview_row,
                        self.rename_plan.as_ref().map_or(0, |plan| plan.moves.len()),
                        true,
                    );
                }
                _ => {}
            },
            Mode::ConfirmRename => {
                if action == Action::Confirm {
                    self.start_rename(sender.clone());
                } else {
                    self.mode = Mode::RenameReview;
                }
            }
            Mode::ConfirmRenameUndo(id) => {
                let id = id.clone();
                self.mode = Mode::Normal;
                if action == Action::Confirm {
                    let root = self.root.clone();
                    self.status = "Restoring paths from rename journal…".into();
                    self.launch_job(JobKind::Recovery, sender.clone(), move |_| {
                        let result = rename::undo(&root, &id).map_err(|error| format!("{error:#}"));
                        Message::RenameUndone(result)
                    });
                }
            }
            Mode::Search(input) => match action {
                Action::Open => {
                    self.filter = input.clone();
                    self.mode = Mode::Normal;
                    self.rebuild();
                }
                Action::Back => self.mode = Mode::Normal,
                Action::DeleteBackward => {
                    input.pop();
                }
                Action::Input(c) => input.push(c),
                _ => {}
            },
            Mode::BatchEdit | Mode::BatchEditInput(_) => self.update_batch_editor(action),
            Mode::Edit(input) => match action {
                Action::Open => {
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
                Action::Back => self.mode = Mode::Normal,
                Action::DeleteBackward => {
                    input.pop();
                }
                Action::Input(c) => input.push(c),
                _ => {}
            },
            Mode::Confirm => {
                self.mode = Mode::Normal;
                if action == Action::Confirm && !self.jobs.is_busy() {
                    let root = self.root.clone();
                    self.status = "Applying staged edits…".into();
                    self.launch_job(JobKind::Apply, sender.clone(), move |_| {
                        Message::Applied(changes::apply(&root))
                    });
                }
            }
            Mode::ConfirmUndo(id) => {
                let id = id.clone();
                self.mode = Mode::Normal;
                if action == Action::Confirm {
                    match changes::undo(&self.root, &id) {
                        Ok(()) => {
                            self.start_scan(sender.clone());
                            self.status = format!("Undid batch {id}; rescanning…");
                        }
                        Err(error) => self.status = format!("Undo failed: {error:#}"),
                    }
                }
            }
            Mode::Help if action == Action::Back => self.mode = Mode::Normal,
            Mode::Help => {}
            Mode::Normal => return self.normal_action(action, sender),
        }
        false
    }
}
