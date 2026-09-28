mod action;
mod app;
mod dialogs;
mod event;
mod keymap;
mod message;
mod navigation;
mod state;
mod views;
mod workflows;

#[cfg(test)]
mod tests;

use action::Action;
use message::*;
use state::*;
use views::render;

use crate::changes::{self, Pending};
use crate::domain::{Edit, Field, Issue, RawValue, Snapshot, Track, TrackId};
use crate::{duplicates, export, library::Index, online, quarantine, rename, rules, tags};
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
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
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
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        previous(info);
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
            let applied = matches!(
                message,
                Message::Applied(Ok(_))
                    | Message::Quarantined(Ok(_))
                    | Message::Restored(Ok(()))
                    | Message::Renamed(_)
                    | Message::RenameUndone(Ok(()))
            );
            app.message(message);
            if applied {
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
