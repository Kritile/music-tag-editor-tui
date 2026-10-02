mod action;
mod app;
mod dialogs;
mod event;
mod job;
mod keymap;
mod message;
mod navigation;
mod state;
mod views;
mod workflows;

#[cfg(test)]
mod tests;

use action::Action;
use job::{JobEvent, JobKind, JobManager};
use message::*;
use state::*;
use views::render;

use crate::changes::{self, Pending};
use crate::domain::{Edit, Field, Issue, RawValue, Snapshot, Track, TrackId};
use crate::{
    duplicates, export, history,
    library::{
        Index,
        watch::{LibraryWatcher, WatchBatch},
    },
    online, quarantine, rename, rules, tags,
};
use anyhow::Result;
use crossterm::event::{self as terminal_event, Event, KeyEvent};
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

#[derive(Default)]
struct PendingWatch {
    paths: BTreeSet<PathBuf>,
    full_rescan: bool,
}

enum WatchWork {
    FullRescan,
    Paths(Vec<PathBuf>),
}

impl PendingWatch {
    fn add(&mut self, batch: WatchBatch) {
        if batch.full_rescan {
            self.full_rescan = true;
            self.paths.clear();
        } else if !self.full_rescan {
            self.paths.extend(batch.paths);
            if self.paths.len() > 1_000 {
                self.full_rescan = true;
                self.paths.clear();
            }
        }
    }

    fn take_if_idle(&mut self, busy: bool) -> Option<WatchWork> {
        if busy {
            return None;
        }
        if std::mem::take(&mut self.full_rescan) {
            return Some(WatchWork::FullRescan);
        }
        (!self.paths.is_empty())
            .then(|| WatchWork::Paths(std::mem::take(&mut self.paths).into_iter().collect()))
    }

    fn restore(&mut self, work: WatchWork) {
        match work {
            WatchWork::FullRescan => self.add(WatchBatch {
                paths: vec![],
                full_rescan: true,
            }),
            WatchWork::Paths(paths) => self.add(WatchBatch {
                paths,
                full_rescan: false,
            }),
        }
    }
}

pub fn run(root: &Path) -> Result<()> {
    run_with_watch(root, true, Duration::from_millis(500))
}

/// Runs the TUI with optional debounced filesystem updates.
pub fn run_with_watch(root: &Path, watch: bool, debounce: Duration) -> Result<()> {
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let previous = std::panic::take_hook();
    let ui_thread = std::thread::current().id();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() == ui_thread {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            previous(info);
        } else if !std::thread::current()
            .name()
            .is_some_and(|name| name.starts_with("music-tui-worker-"))
        {
            previous(info);
        }
    }));
    let mut terminal: Terminal<CrosstermBackend<Stdout>> =
        Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let (sender, receiver): (Sender<Message>, Receiver<Message>) = mpsc::channel();
    let mut app = App::new(root)?;
    let mut watcher = if watch {
        match LibraryWatcher::new(root, debounce) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                app.post_scan_notice =
                    Some(format!("watcher unavailable: {error}; press r to rescan"));
                None
            }
        }
    } else {
        None
    };
    app.start_scan(sender.clone());
    let mut pending_watch = PendingWatch::default();
    loop {
        for _ in 0..100 {
            let Ok(message) = receiver.try_recv() else {
                break;
            };
            let applied = message.requests_rescan();
            let accepted = app.message(message);
            if applied && accepted {
                app.start_scan(sender.clone());
            }
        }
        if let Some(batch) = watcher.as_mut().and_then(LibraryWatcher::poll_due) {
            pending_watch.add(batch);
        }
        // Only the job manager opens the index for watcher updates, so tag writes,
        // renames, and a full scan cannot race with an incremental refresh.
        if let Some(work) = pending_watch.take_if_idle(app.jobs.is_busy()) {
            match work {
                WatchWork::FullRescan => {
                    app.start_scan(sender.clone());
                    if !app.jobs.is_busy() {
                        pending_watch.restore(WatchWork::FullRescan);
                    }
                }
                WatchWork::Paths(paths) => {
                    if !app.start_incremental_refresh(paths.clone(), sender.clone()) {
                        pending_watch.restore(WatchWork::Paths(paths));
                    }
                }
            }
        }
        if app.tracks_dirty {
            app.rebuild();
            app.tracks_dirty = false;
        }
        terminal.draw(|frame| render(frame, &app))?;
        if terminal_event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = terminal_event::read()?
            && app.key(key, &sender)
        {
            break;
        }
    }
    Ok(())
}
