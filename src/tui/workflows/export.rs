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
        self.mode = Mode::Normal;
        self.status = "Checking export destination and source files…".into();
        self.launch_job(JobKind::ExportPreview, sender, move |_| {
            Message::ExportPlanned(export::plan(&root, Path::new(&destination), &files))
        });
    }

    pub(in crate::tui) fn start_export(&mut self, sender: Sender<Message>) {
        let Some(plan) = self.export_plan.clone() else {
            return;
        };
        self.status = "Exporting verified copies…".into();
        self.mode = Mode::Normal;
        self.launch_job(JobKind::Export, sender, move |job| {
            let result = export::run(&plan, |count, path| {
                job.progress(Message::ExportProgress(count, path.to_path_buf()));
                !job.cancelled()
            });
            Message::Exported(result)
        });
    }
}
