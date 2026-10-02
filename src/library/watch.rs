//! Debounced filesystem notifications. The watcher only reports paths; callers
//! apply them after their own file operations have finished.

use crate::{quarantine, tags};
use notify::{
    Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{CreateKind, ModifyKind, RemoveKind},
};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

/// Files changed during one quiet period. A rescan supersedes individual paths.
#[derive(Debug)]
pub struct WatchBatch {
    pub paths: Vec<PathBuf>,
    pub full_rescan: bool,
}

/// Owns a recursive OS watcher. Poll it from the application's event loop.
pub struct LibraryWatcher {
    _watcher: RecommendedWatcher,
    receiver: Receiver<notify::Result<Event>>,
    batcher: EventBatcher,
}

impl LibraryWatcher {
    pub fn new(root: &Path, debounce: Duration) -> notify::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let mut watcher = RecommendedWatcher::new(
            move |event| {
                let _ = sender.send(event);
            },
            Config::default(),
        )?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self {
            _watcher: watcher,
            receiver,
            batcher: EventBatcher::new(root.to_path_buf(), debounce),
        })
    }

    /// Drains pending notifications and returns a batch after the quiet period.
    pub fn poll_due(&mut self) -> Option<WatchBatch> {
        let now = Instant::now();
        for event in self.receiver.try_iter() {
            match event {
                Ok(event) => self.batcher.record(event, now),
                Err(_) => self.batcher.request_rescan(now),
            }
        }
        self.batcher.take_due(now)
    }
}

struct EventBatcher {
    root: PathBuf,
    debounce: Duration,
    deadline: Option<Instant>,
    paths: BTreeSet<PathBuf>,
    full_rescan: bool,
}

impl EventBatcher {
    fn new(root: PathBuf, debounce: Duration) -> Self {
        Self {
            root,
            debounce,
            deadline: None,
            paths: BTreeSet::new(),
            full_rescan: false,
        }
    }

    fn request_rescan(&mut self, now: Instant) {
        self.full_rescan = true;
        self.deadline = Some(now + self.debounce);
    }

    fn record(&mut self, event: Event, now: Instant) {
        if event.need_rescan() {
            self.request_rescan(now);
            return;
        }
        match event.kind {
            EventKind::Access(_) => return,
            EventKind::Create(CreateKind::Folder)
            | EventKind::Remove(RemoveKind::Folder)
            | EventKind::Any => {
                self.request_rescan(now);
                return;
            }
            EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_) => {}
            _ => return,
        }
        if event.paths.is_empty() {
            self.request_rescan(now);
            return;
        }
        let rename = matches!(event.kind, EventKind::Modify(ModifyKind::Name(_)));
        let removal = matches!(event.kind, EventKind::Remove(_));
        for path in event.paths {
            let Ok(relative) = path.strip_prefix(&self.root) else {
                continue;
            };
            if relative.as_os_str().is_empty()
                || relative
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
                || relative
                    .components()
                    .next()
                    .is_some_and(|part| part.as_os_str() == quarantine::FOLDER)
                || path.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    name.starts_with(".music-tui-") || name.starts_with(".tmp")
                })
            {
                continue;
            }
            if path.is_dir() || ((rename || removal) && tags::format(&path).is_none()) {
                self.request_rescan(now);
            } else if tags::format(&path).is_some() {
                self.paths.insert(path);
                self.deadline = Some(now + self.debounce);
            }
        }
    }

    fn take_due(&mut self, now: Instant) -> Option<WatchBatch> {
        if !self.deadline.is_some_and(|deadline| deadline <= now) {
            return None;
        }
        self.deadline = None;
        let full_rescan = std::mem::take(&mut self.full_rescan);
        let paths = std::mem::take(&mut self.paths).into_iter().collect();
        Some(WatchBatch { paths, full_rescan })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};

    #[test]
    fn coalesces_renames_and_repeated_modifications_until_debounce_expires() {
        let root = Path::new("/music");
        let start = Instant::now();
        let mut batcher = EventBatcher::new(root.to_path_buf(), Duration::from_millis(200));
        batcher.record(
            Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
                .add_path(root.join("old.mp3"))
                .add_path(root.join("new.mp3")),
            start,
        );
        batcher.record(
            Event::new(EventKind::Create(CreateKind::File)).add_path(root.join("new.mp3")),
            start + Duration::from_millis(100),
        );
        assert!(
            batcher
                .take_due(start + Duration::from_millis(250))
                .is_none()
        );
        let batch = batcher
            .take_due(start + Duration::from_millis(300))
            .expect("debounced batch");
        assert!(!batch.full_rescan);
        assert_eq!(
            batch.paths,
            vec![root.join("new.mp3"), root.join("old.mp3")]
        );
    }

    #[test]
    fn directory_change_requests_full_rescan_and_temporary_file_is_ignored() {
        let root = Path::new("/music");
        let start = Instant::now();
        let mut batcher = EventBatcher::new(root.to_path_buf(), Duration::from_millis(50));
        batcher.record(
            Event::new(EventKind::Create(CreateKind::File))
                .add_path(root.join(".music-tui-write.mp3")),
            start,
        );
        assert!(
            batcher
                .take_due(start + Duration::from_millis(50))
                .is_none()
        );
        batcher.record(
            Event::new(EventKind::Create(CreateKind::Folder)).add_path(root.join("album")),
            start,
        );
        assert!(
            batcher
                .take_due(start + Duration::from_millis(50))
                .expect("batch")
                .full_rescan
        );
    }

    #[test]
    fn ambiguous_removed_directory_requests_full_rescan() {
        let root = Path::new("/music");
        let start = Instant::now();
        let mut batcher = EventBatcher::new(root.to_path_buf(), Duration::from_millis(50));
        batcher.record(
            Event::new(EventKind::Remove(RemoveKind::Any)).add_path(root.join("album")),
            start,
        );
        assert!(
            batcher
                .take_due(start + Duration::from_millis(50))
                .expect("batch")
                .full_rescan
        );
    }

    #[test]
    fn pathless_modify_requests_full_rescan() {
        let root = Path::new("/music");
        let start = Instant::now();
        let mut batcher = EventBatcher::new(root.to_path_buf(), Duration::from_millis(50));
        batcher.record(Event::new(EventKind::Modify(ModifyKind::Any)), start);
        assert!(
            batcher
                .take_due(start + Duration::from_millis(50))
                .expect("batch")
                .full_rescan
        );
    }
}
