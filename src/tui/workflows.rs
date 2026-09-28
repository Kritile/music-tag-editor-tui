use super::*;

impl App {
    pub(super) fn start_rename_preview(&mut self, template: String, sender: Sender<Message>) {
        let root = self.root.clone();
        let tracks: Vec<_> = if !self.selected.is_empty() {
            self.tracks
                .iter()
                .filter(|track| self.selected.contains(&track.id))
                .cloned()
                .collect()
        } else if let Some(key) = self
            .selected_album
            .clone()
            .or_else(|| self.current().map(rules::album_key))
        {
            self.tracks
                .iter()
                .filter(|track| rules::album_key(track) == key)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        if tracks.is_empty() {
            self.status = "Select tracks or open an album before renaming".into();
            self.mode = Mode::Normal;
            return;
        }
        self.busy = true;
        self.mode = Mode::Normal;
        self.status = "Checking rename paths and file fingerprints…".into();
        thread::spawn(move || {
            let result =
                rename::plan(&root, &tracks, &template).map_err(|error| format!("{error:#}"));
            let _ = sender.send(Message::RenamePlanned(result));
        });
    }

    pub(super) fn start_rename(&mut self, sender: Sender<Message>) {
        let Some(plan) = self.rename_plan.clone() else {
            return;
        };
        let root = self.root.clone();
        self.busy = true;
        self.mode = Mode::Normal;
        self.status = "Renaming files with recovery journal…".into();
        thread::spawn(move || {
            let result = rename::run(&root, &plan).map_err(|error| format!("{error:#}"));
            let _ = sender.send(Message::Renamed(result));
        });
    }

    pub(super) fn start_export_preview(&mut self, destination: String, sender: Sender<Message>) {
        let root = self.root.clone();
        let files: Vec<_> = self
            .tracks
            .iter()
            .map(|track| track.snapshot.path.clone())
            .collect();
        if files.is_empty() {
            self.status = "No indexed tracks to export".into();
            self.mode = Mode::Normal;
            return;
        }
        self.busy = true;
        self.mode = Mode::Normal;
        self.status = "Checking export destination and source files…".into();
        thread::spawn(move || {
            let result = export::plan(&root, Path::new(&destination), &files)
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(Message::ExportPlanned(result));
        });
    }

    pub(super) fn start_export(&mut self, sender: Sender<Message>) {
        let Some(plan) = self.export_plan.clone() else {
            return;
        };
        self.cancel_export = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&self.cancel_export);
        self.busy = true;
        self.status = "Exporting verified copies…".into();
        self.mode = Mode::Normal;
        thread::spawn(move || {
            let result = export::run(&plan, |count, path| {
                let _ = sender.send(Message::ExportProgress(count, path.to_path_buf()));
                !cancelled.load(Ordering::Relaxed)
            })
            .map_err(|error| format!("{error:#}"));
            let _ = sender.send(Message::Exported(result));
        });
    }

    pub(super) fn start_online_search(&mut self, sender: Sender<Message>) {
        let key = self
            .selected_album
            .clone()
            .or_else(|| self.current().map(rules::album_key));
        let Some(key) = key else {
            self.status = "Open an album before searching MusicBrainz".into();
            return;
        };
        let local: Vec<_> = self
            .tracks
            .iter()
            .filter(|track| rules::album_key(track) == key)
            .cloned()
            .collect();
        let Some(first) = local.first() else {
            return;
        };
        let artist = artist_name(first).to_string();
        let album = album_name(first).to_string();
        self.online = Some(OnlineState::new(local));
        self.online_generation += 1;
        let generation = self.online_generation;
        self.busy = true;
        self.status = format!("Searching MusicBrainz: {artist} — {album}");
        thread::spawn(move || {
            let result = online::Client::new()
                .and_then(|mut client| client.search(&artist, &album))
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(Message::OnlineCandidates(generation, result));
        });
    }

    pub(super) fn load_online_release(&mut self, sender: Sender<Message>) {
        let Some(id) = self
            .online
            .as_ref()
            .and_then(|state| state.candidates.get(state.candidate_row))
            .map(|candidate| candidate.id.clone())
        else {
            return;
        };
        let generation = self.online_generation;
        self.busy = true;
        self.status = "Loading MusicBrainz release…".into();
        thread::spawn(move || {
            let result = online::Client::new()
                .and_then(|mut client| client.release(&id))
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(Message::OnlineRelease(generation, result));
        });
    }

    pub(super) fn stage_online_proposals(&mut self) {
        let Some(state) = self.online.as_ref() else {
            return;
        };
        let chosen: Vec<_> = state
            .proposals
            .iter()
            .zip(&state.checked)
            .filter(|(_, selected)| **selected)
            .map(|(proposal, _)| proposal.clone())
            .collect();
        let mut staged_count = 0;
        for proposal in chosen {
            let Some(track) = self
                .tracks
                .iter()
                .find(|track| track.id == proposal.local_id)
            else {
                continue;
            };
            match changes::stage(&self.root, track, proposal.edit) {
                Ok(staged) => {
                    self.staged = staged;
                    staged_count += 1;
                }
                Err(error) => {
                    self.status = format!("Staged {staged_count} edits; next failed: {error:#}");
                    return;
                }
            }
        }
        self.mode = Mode::Normal;
        self.status =
            format!("Staged {staged_count} MusicBrainz suggestions; review diff before Apply");
    }
}

pub(super) fn render_online(frame: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.online else {
        return;
    };
    let (title, lines, selected): (String, Vec<String>, usize) = match app.mode {
        Mode::OnlineCandidates => (
            format!("MusicBrainz releases ({})", state.candidates.len()),
            state
                .candidates
                .iter()
                .map(|candidate| {
                    format!(
                        "{} — {} | {} {} | {} tracks",
                        candidate.artist,
                        candidate.title,
                        candidate.date.as_deref().unwrap_or("?"),
                        candidate.country.as_deref().unwrap_or("?"),
                        candidate.tracks
                    )
                })
                .collect(),
            state.candidate_row,
        ),
        Mode::OnlineMatches => {
            let Some(release) = &state.release else {
                return;
            };
            let lines = state
                .local
                .iter()
                .enumerate()
                .map(|(index, track)| {
                    let matched = state
                        .mapping
                        .get(index)
                        .and_then(|item| *item)
                        .and_then(|remote| release.tracks.get(remote));
                    format!(
                        "{} → {}",
                        track.metadata.title.as_deref().unwrap_or("?"),
                        matched.map_or("unmatched".to_string(), |item| format!(
                            "{}-{} {}",
                            item.disc, item.number, item.title
                        ))
                    )
                })
                .collect();
            let remote = release
                .tracks
                .get(state.remote_row)
                .map(|track| {
                    format!(
                        "{}-{} {} — {}",
                        track.disc, track.number, track.title, track.artist
                    )
                })
                .unwrap_or_else(|| "no remote track".into());
            (
                format!("{} — {} | remote: {remote}", release.artist, release.title),
                lines,
                state.local_row,
            )
        }
        Mode::OnlineReview => (
            format!("Review MusicBrainz suggestions ({})", state.proposals.len()),
            state
                .proposals
                .iter()
                .zip(&state.checked)
                .map(|(proposal, selected)| {
                    let track = state
                        .local
                        .iter()
                        .find(|track| track.id == proposal.local_id);
                    format!(
                        "[{}] {} | {}: {:?} → {}",
                        if *selected { 'x' } else { ' ' },
                        track.map_or("?", |track| track
                            .snapshot
                            .path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("?")),
                        proposal.edit.field.label(),
                        proposal.before,
                        proposal.edit.value
                    )
                })
                .collect(),
            state.proposal_row,
        ),
        _ => return,
    };
    let height = area.height.saturating_sub(2) as usize;
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
