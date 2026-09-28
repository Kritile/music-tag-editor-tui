use super::*;

impl App {
    pub(super) fn update(&mut self, action: Action, sender: &Sender<Message>) -> bool {
        match &mut self.mode {
            Mode::Actions(index) => match action {
                Action::Cancel => self.mode = Mode::Normal,
                Action::MoveUp | Action::Character('k') => {
                    *index = wrapped_index(*index, ACTIONS.len(), false)
                }
                Action::MoveDown | Action::Character('j') => {
                    *index = wrapped_index(*index, ACTIONS.len(), true)
                }
                Action::Activate => {
                    let chosen = *index;
                    self.action(chosen, sender);
                }
                _ => {}
            },
            Mode::Palette(input, index) => match action {
                Action::Cancel => self.mode = Mode::Normal,
                Action::DeleteBackward => {
                    input.pop();
                    *index = 0;
                }
                Action::Character(c) => {
                    input.push(c);
                    *index = 0;
                }
                Action::MoveUp => {
                    *index = wrapped_index(*index, matching_actions(input).len(), false)
                }
                Action::MoveDown => {
                    *index = wrapped_index(*index, matching_actions(input).len(), true)
                }
                Action::Activate => {
                    let matches = matching_actions(input);
                    if let Some(&chosen) = matches.get(*index) {
                        self.action(chosen, sender);
                    }
                }
                _ => {}
            },
            Mode::CheckScope(index) => match action {
                Action::Cancel => self.mode = Mode::Normal,
                Action::MoveUp | Action::Character('k') => {
                    *index = wrapped_index(*index, SCOPES.len(), false)
                }
                Action::MoveDown | Action::Character('j') => {
                    *index = wrapped_index(*index, SCOPES.len(), true)
                }
                Action::Activate => {
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
            Mode::Results => match action {
                Action::Cancel | Action::Character('q') => self.mode = Mode::Normal,
                Action::Character('f') => {
                    self.issue_filter = self.issue_filter.next();
                    self.issue_row = 0;
                }
                Action::MoveDown | Action::Character('j') => {
                    self.issue_row =
                        wrapped_index(self.issue_row, self.visible_issues().len(), true)
                }
                Action::MoveUp | Action::Character('k') => {
                    self.issue_row =
                        wrapped_index(self.issue_row, self.visible_issues().len(), false)
                }
                Action::Activate => {
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
                Action::Character('s') => self.stage_suggestion(),
                _ => {}
            },
            Mode::DiffReview => match action {
                Action::Cancel => self.mode = Mode::Normal,
                Action::MoveDown | Action::Character('j') => {
                    self.review_row = wrapped_index(self.review_row, self.review_lines.len(), true)
                }
                Action::MoveUp | Action::Character('k') => {
                    self.review_row = wrapped_index(self.review_row, self.review_lines.len(), false)
                }
                Action::Character('a')
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
            Mode::Duplicates => match action {
                Action::Cancel | Action::Character('q') => self.mode = Mode::Normal,
                Action::MoveDown | Action::Character('j') => {
                    self.duplicate_group = wrapped_index(
                        self.duplicate_group,
                        self.duplicate_report.as_ref().map_or(0, |r| r.groups.len()),
                        true,
                    );
                }
                Action::MoveUp | Action::Character('k') => {
                    self.duplicate_group = wrapped_index(
                        self.duplicate_group,
                        self.duplicate_report.as_ref().map_or(0, |r| r.groups.len()),
                        false,
                    );
                }
                Action::Activate
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
                Action::Cancel => self.mode = Mode::Duplicates,
                Action::MoveDown | Action::Character('j') => {
                    self.duplicate_member = wrapped_index(
                        self.duplicate_member,
                        self.duplicate_report
                            .as_ref()
                            .and_then(|r| r.groups.get(self.duplicate_group))
                            .map_or(0, |g| g.members.len()),
                        true,
                    );
                }
                Action::MoveUp | Action::Character('k') => {
                    self.duplicate_member = wrapped_index(
                        self.duplicate_member,
                        self.duplicate_report
                            .as_ref()
                            .and_then(|r| r.groups.get(self.duplicate_group))
                            .map_or(0, |g| g.members.len()),
                        false,
                    );
                }
                Action::Character('x') if !self.busy => {
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
                if action == Action::Character('y')
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
            Mode::QuarantineHistory => match action {
                Action::Cancel => self.mode = Mode::Normal,
                Action::MoveDown | Action::Character('j') => {
                    self.quarantine_row =
                        wrapped_index(self.quarantine_row, self.quarantine_records.len(), true);
                }
                Action::MoveUp | Action::Character('k') => {
                    self.quarantine_row =
                        wrapped_index(self.quarantine_row, self.quarantine_records.len(), false);
                }
                Action::Activate => {
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
                if action == Action::Character('y')
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
            Mode::OnlineCandidates => match action {
                Action::Cancel => {
                    if self.busy {
                        self.online_generation += 1;
                        self.busy = false;
                    }
                    self.mode = Mode::Normal;
                }
                Action::MoveUp | Action::Character('k') => {
                    if let Some(state) = &mut self.online {
                        state.candidate_row =
                            wrapped_index(state.candidate_row, state.candidates.len(), false);
                    }
                }
                Action::MoveDown | Action::Character('j') => {
                    if let Some(state) = &mut self.online {
                        state.candidate_row =
                            wrapped_index(state.candidate_row, state.candidates.len(), true);
                    }
                }
                Action::Activate if !self.busy => self.load_online_release(sender.clone()),
                _ => {}
            },
            Mode::OnlineMatches => match action {
                Action::Cancel => self.mode = Mode::OnlineCandidates,
                Action::MoveUp | Action::Character('k') => {
                    if let Some(state) = &mut self.online {
                        state.local_row = wrapped_index(state.local_row, state.local.len(), false);
                    }
                }
                Action::MoveDown | Action::Character('j') => {
                    if let Some(state) = &mut self.online {
                        state.local_row = wrapped_index(state.local_row, state.local.len(), true);
                    }
                }
                Action::MoveLeft | Action::Character('h') => {
                    if let Some(state) = &mut self.online {
                        let len = state.release.as_ref().map_or(0, |r| r.tracks.len());
                        state.remote_row = wrapped_index(state.remote_row, len, false);
                    }
                }
                Action::MoveRight | Action::Character('l') => {
                    if let Some(state) = &mut self.online {
                        let len = state.release.as_ref().map_or(0, |r| r.tracks.len());
                        state.remote_row = wrapped_index(state.remote_row, len, true);
                    }
                }
                Action::Activate => {
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
                Action::Character('x') => {
                    if let Some(state) = &mut self.online
                        && let Some(mapped) = state.mapping.get_mut(state.local_row)
                    {
                        *mapped = None;
                    }
                }
                Action::Character('r') => {
                    if let Some(state) = &mut self.online {
                        state.build_proposals();
                        self.mode = Mode::OnlineReview;
                    }
                }
                _ => {}
            },
            Mode::OnlineReview => match action {
                Action::Cancel => self.mode = Mode::OnlineMatches,
                Action::MoveUp | Action::Character('k') => {
                    if let Some(state) = &mut self.online {
                        state.proposal_row =
                            wrapped_index(state.proposal_row, state.proposals.len(), false);
                    }
                }
                Action::MoveDown | Action::Character('j') => {
                    if let Some(state) = &mut self.online {
                        state.proposal_row =
                            wrapped_index(state.proposal_row, state.proposals.len(), true);
                    }
                }
                Action::Character(' ') => {
                    if let Some(state) = &mut self.online
                        && let Some(checked) = state.checked.get_mut(state.proposal_row)
                    {
                        *checked = !*checked;
                    }
                }
                Action::Character('s') => self.stage_online_proposals(),
                _ => {}
            },
            Mode::ExportPath(input) => match action {
                Action::Cancel => self.mode = Mode::Normal,
                Action::DeleteBackward => {
                    input.pop();
                }
                Action::Character(ch) => input.push(ch),
                Action::Activate => {
                    let destination = input.clone();
                    self.start_export_preview(destination, sender.clone());
                }
                _ => {}
            },
            Mode::ExportReview => match action {
                Action::Cancel => self.mode = Mode::Normal,
                Action::Activate => self.mode = Mode::ConfirmExport,
                Action::MoveUp | Action::Character('k') => {
                    self.preview_row = wrapped_index(
                        self.preview_row,
                        self.export_plan
                            .as_ref()
                            .map_or(0, |plan| plan.entries.len()),
                        false,
                    );
                }
                Action::MoveDown | Action::Character('j') => {
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
                if action == Action::Character('y') {
                    self.start_export(sender.clone());
                } else {
                    self.mode = Mode::ExportReview;
                }
            }
            Mode::RenameTemplate(input) => match action {
                Action::Cancel => self.mode = Mode::Normal,
                Action::DeleteBackward => {
                    input.pop();
                }
                Action::Character(ch) => input.push(ch),
                Action::Activate => {
                    let template = input.clone();
                    self.start_rename_preview(template, sender.clone());
                }
                _ => {}
            },
            Mode::RenameReview => match action {
                Action::Cancel => self.mode = Mode::Normal,
                Action::Activate => self.mode = Mode::ConfirmRename,
                Action::MoveUp | Action::Character('k') => {
                    self.preview_row = wrapped_index(
                        self.preview_row,
                        self.rename_plan.as_ref().map_or(0, |plan| plan.moves.len()),
                        false,
                    );
                }
                Action::MoveDown | Action::Character('j') => {
                    self.preview_row = wrapped_index(
                        self.preview_row,
                        self.rename_plan.as_ref().map_or(0, |plan| plan.moves.len()),
                        true,
                    );
                }
                _ => {}
            },
            Mode::ConfirmRename => {
                if action == Action::Character('y') {
                    self.start_rename(sender.clone());
                } else {
                    self.mode = Mode::RenameReview;
                }
            }
            Mode::ConfirmRenameUndo(id) => {
                let id = id.clone();
                self.mode = Mode::Normal;
                if action == Action::Character('y') {
                    let root = self.root.clone();
                    let sender = sender.clone();
                    self.busy = true;
                    self.status = "Restoring paths from rename journal…".into();
                    thread::spawn(move || {
                        let result = rename::undo(&root, &id).map_err(|error| format!("{error:#}"));
                        let _ = sender.send(Message::RenameUndone(result));
                    });
                }
            }
            Mode::Search(input) => match action {
                Action::Activate => {
                    self.filter = input.clone();
                    self.mode = Mode::Normal;
                    self.rebuild();
                }
                Action::Cancel => self.mode = Mode::Normal,
                Action::DeleteBackward => {
                    input.pop();
                }
                Action::Character(c) => input.push(c),
                _ => {}
            },
            Mode::Edit(input) => match action {
                Action::Activate => {
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
                Action::Cancel => self.mode = Mode::Normal,
                Action::DeleteBackward => {
                    input.pop();
                }
                Action::Character(c) => input.push(c),
                _ => {}
            },
            Mode::Confirm => {
                self.mode = Mode::Normal;
                if action == Action::Character('y') && !self.busy {
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
                if action == Action::Character('y') {
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
            Mode::Normal => return self.normal_action(action, sender),
        }
        false
    }
}
