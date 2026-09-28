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
use crate::{duplicates, export, history, library::Index, online, quarantine, rename, rules, tags};
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

pub fn run(root: &Path) -> Result<()> {
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
    app.start_scan(sender.clone());
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
