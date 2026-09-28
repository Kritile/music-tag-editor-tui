use super::super::*;

impl App {
    pub(in crate::tui) fn start_rename_preview(
        &mut self,
        template: String,
        sender: Sender<Message>,
    ) {
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
        self.mode = Mode::Normal;
        self.status = "Checking rename paths and file fingerprints…".into();
        self.launch_job(JobKind::RenamePreview, sender, move |_| {
            Message::RenamePlanned(rename::plan(&root, &tracks, &template))
        });
    }

    pub(in crate::tui) fn start_rename(&mut self, sender: Sender<Message>) {
        let Some(plan) = self.rename_plan.clone() else {
            return;
        };
        let root = self.root.clone();
        self.mode = Mode::Normal;
        self.status = "Renaming files with recovery journal…".into();
        self.launch_job(JobKind::Rename, sender, move |_| {
            Message::Renamed(rename::run(&root, &plan))
        });
    }
}
