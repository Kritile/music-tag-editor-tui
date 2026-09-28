use super::super::*;

impl App {
    pub(in crate::tui) fn start_export_preview(
        &mut self,
        destination: String,
        sender: Sender<Message>,
    ) {
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
            let _ = sender.send(Message::ExportPlanned(export::plan(
                &root,
                Path::new(&destination),
                &files,
            )));
        });
    }

    pub(in crate::tui) fn start_export(&mut self, sender: Sender<Message>) {
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
            });
            let _ = sender.send(Message::Exported(result));
        });
    }
}
