use super::*;

impl App {
    pub(super) fn group_for(&self, track: &Track) -> String {
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

    pub(super) fn folder_for_check(&self) -> Option<PathBuf> {
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

    pub(super) fn visible_issues(&self) -> Vec<&Issue> {
        self.issues
            .iter()
            .filter(|issue| match self.issue_filter {
                IssueFilter::All => true,
                IssueFilter::Generic => !issue.rule_id.starts_with("echo_"),
                IssueFilter::Echo => issue.rule_id.starts_with("echo_"),
            })
            .collect()
    }

    pub(super) fn rebuild(&mut self) {
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
            if !self.jobs.is_busy()
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
                if !self.jobs.is_busy()
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
            if !self.jobs.is_busy()
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
            if matched_index.is_some() || !self.jobs.is_busy() || previous_key.is_none() {
                self.cursor_key = Some(key.clone());
            }
        } else if !self.jobs.is_busy() {
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
        if !self.jobs.is_busy() || self.row_anchor.is_none() {
            self.row_anchor = self.current().map(|track| track.id);
        }
    }

    pub(super) fn current(&self) -> Option<&Track> {
        self.visible.get(self.row).and_then(|&i| self.tracks.get(i))
    }

    pub(super) fn move_cursor(&mut self, down: bool) {
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

    pub(super) fn normal_action(&mut self, action: Action, sender: &Sender<Message>) -> bool {
        match action {
            Action::Quit if !self.jobs.is_busy() => return true,
            Action::Help => self.mode = Mode::Help,
            Action::MoveDown => self.move_cursor(true),
            Action::MoveUp => self.move_cursor(false),
            Action::NextFocus => {
                self.focus = match self.focus {
                    Focus::Tree => Focus::Tracks,
                    Focus::Tracks => Focus::Inspector,
                    Focus::Inspector => Focus::Tree,
                }
            }
            Action::CycleGrouping => {
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
            Action::Open if self.focus == Focus::Tree => {
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
            Action::Back if !self.jobs.is_busy() => return true,
            Action::DeleteBackward => {
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
            Action::Search => self.mode = Mode::Search(self.filter.clone()),
            Action::SelectCurrent => {
                if let Some(id) = self.current().map(|t| t.id)
                    && !self.selected.insert(id)
                {
                    self.selected.remove(&id);
                }
            }
            Action::SelectVisible => {
                for &index in &self.visible {
                    self.selected.insert(self.tracks[index].id);
                }
                self.status = format!("Selected {} visible tracks", self.visible.len());
            }
            Action::ClearSelection => {
                self.selected.clear();
                self.status = "Selection cleared".into();
            }
            Action::Edit => self.mode = Mode::Edit(String::new()),
            Action::BatchEdit => self.open_batch_editor(),
            Action::StageSuggestion => self.stage_suggestion(),
            Action::ReviewDiff => {
                self.tab = Tab::Diff;
                self.inspector_scroll = 0;
                self.open_diff_review();
            }
            Action::ShowNormalized => {
                self.tab = Tab::Normalized;
                self.inspector_scroll = 0;
            }
            Action::ShowRaw => {
                self.tab = Tab::Raw;
                self.inspector_scroll = 0;
            }
            Action::ShowDevice => {
                self.tab = Tab::Device;
                self.inspector_scroll = 0;
            }
            Action::Apply if !self.staged.is_empty() => self.open_diff_review(),
            Action::Refresh if !self.jobs.is_busy() => self.start_scan(sender.clone()),
            Action::Check => self.mode = Mode::CheckScope(0),
            Action::OpenActions => self.mode = Mode::Actions(0),
            Action::OpenCommandPalette => self.mode = Mode::Palette(String::new(), 0),
            Action::CancelWork if self.jobs.active_kind() == Some(JobKind::Scan) => {
                self.jobs.cancel(false);
                self.status = "Cancelling scan after current file…".into();
            }
            Action::CancelWork if self.jobs.active_kind() == Some(JobKind::HashDuplicates) => {
                self.jobs.cancel(false);
                self.status = "Cancelling duplicate search…".into();
            }
            Action::CancelWork
                if matches!(
                    self.jobs.active_kind(),
                    Some(JobKind::MusicBrainzSearch | JobKind::MusicBrainzRelease)
                ) =>
            {
                self.jobs.cancel(true);
                self.status = "MusicBrainz lookup cancelled".into();
            }
            Action::CancelWork if self.jobs.active_kind() == Some(JobKind::Export) => {
                self.jobs.cancel(false);
                self.status = "Stopping export after current file…".into();
            }
            _ => {}
        }
        false
    }
}
