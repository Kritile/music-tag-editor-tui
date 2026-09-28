use super::super::*;

impl App {
    pub(in crate::tui) fn start_online_search(&mut self, sender: Sender<Message>) {
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
        self.status = format!("Searching MusicBrainz: {artist} — {album}");
        self.launch_job(JobKind::MusicBrainzSearch, sender, move |_| {
            let result =
                online::Client::new().and_then(|mut client| client.search(&artist, &album));
            Message::OnlineCandidates(result)
        });
    }

    pub(in crate::tui) fn load_online_release(&mut self, sender: Sender<Message>) {
        let Some(id) = self
            .online
            .as_ref()
            .and_then(|state| state.candidates.get(state.candidate_row))
            .map(|candidate| candidate.id.clone())
        else {
            return;
        };
        self.status = "Loading MusicBrainz release…".into();
        self.launch_job(JobKind::MusicBrainzRelease, sender, move |_| {
            let result = online::Client::new().and_then(|mut client| client.release(&id));
            Message::OnlineRelease(result)
        });
    }

    pub(in crate::tui) fn stage_online_proposals(&mut self) {
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
