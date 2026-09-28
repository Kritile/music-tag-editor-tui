use super::*;

mod duplicates;
mod history;
mod inspector;
mod online;
mod results;
mod tracks;
mod tree;

use duplicates::render_duplicate_screen;
use history::render_history;
use inspector::render_inspector;
use online::render_online;
use results::render_results;
use tracks::render_tracks;
pub(super) use tree::render_tree;

pub(super) fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 12 || area.height < 5 {
        frame.render_widget(Paragraph::new("Enlarge terminal"), area);
        return;
    }
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(area);
    if dialogs::render_modal(frame, [vertical[0], vertical[1], vertical[2]], app) {
        return;
    }
    if matches!(app.mode, Mode::History | Mode::ConfirmHistoryUndo(_)) {
        render_history(frame, vertical[1], app);
        let help = match &app.mode {
            Mode::ConfirmHistoryUndo(key) => format!(
                "Undo {:?} operation {}? y confirms; any other key cancels",
                key.kind, key.id
            ),
            _ => format!(
                "History: j/k select | h/l details | Enter undo when safe | r reload | Esc back | {}",
                app.status
            ),
        };
        frame.render_widget(
            Paragraph::new(help).block(Block::default().borders(Borders::TOP)),
            vertical[2],
        );
        return;
    }
    if matches!(
        app.mode,
        Mode::OnlineCandidates | Mode::OnlineMatches | Mode::OnlineReview
    ) {
        render_online(frame, vertical[1], app);
        let help = match app.mode {
            Mode::OnlineCandidates => "MusicBrainz: j/k choose release | Enter inspect | Esc back",
            Mode::OnlineMatches => {
                "j/k local | h/l remote | Enter map | x unmap | r review fields | Esc back"
            }
            Mode::OnlineReview => "j/k fields | Space choose | s stage chosen | Esc back",
            _ => "",
        };
        frame.render_widget(
            Paragraph::new(help).block(Block::default().borders(Borders::TOP)),
            vertical[2],
        );
        return;
    }
    if matches!(app.mode, Mode::Results) {
        render_results(frame, vertical[1], app);
        frame.render_widget(
            Paragraph::new(
                "Results: j/k move | f filter | Enter inspect | s stage suggestion | Esc back",
            )
            .block(Block::default().borders(Borders::TOP)),
            vertical[2],
        );
        return;
    }
    if matches!(
        app.mode,
        Mode::Duplicates
            | Mode::DuplicateCompare
            | Mode::ConfirmQuarantine(_, _)
            | Mode::QuarantineHistory
            | Mode::ConfirmRestore(_)
    ) {
        render_duplicate_screen(frame, vertical[1], app);
        let help = match app.mode {
            Mode::Duplicates => "Duplicate groups: j/k move | Enter compare | Esc back",
            Mode::DuplicateCompare => "j/k choose file | x quarantine chosen file | Esc groups",
            Mode::QuarantineHistory => "Quarantine: j/k move | Enter restore | Esc back",
            Mode::ConfirmQuarantine(_, _) => {
                "Move selected file to quarantine? y confirms; any other key cancels"
            }
            Mode::ConfirmRestore(_) => "Restore selected file? y confirms; any other key cancels",
            _ => "",
        };
        frame.render_widget(
            Paragraph::new(help).block(Block::default().borders(Borders::TOP)),
            vertical[2],
        );
        return;
    }
    frame.render_widget(
        Paragraph::new(format!(
            "{} | {} | {} selected | {} staged | Actions [m] | Palette [:] | Check [C]",
            app.root.display(),
            app.group_mode.label(),
            app.selected.len(),
            app.staged.len()
        ))
        .block(Block::default().borders(Borders::BOTTOM)),
        vertical[0],
    );
    let narrow = area.width < 90;
    if narrow {
        match app.focus {
            Focus::Tree => render_tree(frame, vertical[1], app),
            Focus::Tracks => render_tracks(frame, vertical[1], app),
            Focus::Inspector => render_inspector(frame, vertical[1], app),
        }
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(22),
                Constraint::Percentage(38),
                Constraint::Percentage(40),
            ])
            .split(vertical[1]);
        render_tree(frame, cols[0], app);
        render_tracks(frame, cols[1], app);
        render_inspector(frame, cols[2], app);
    }
    let status = match &app.mode {
        Mode::Normal => app.status.clone(),
        Mode::Search(s) => format!("Search: {s}"),
        Mode::Edit(s) => format!("Edit field=value: {s}"),
        Mode::ExportPath(s) => format!("Existing export destination directory: {s}"),
        Mode::RenameTemplate(s) => format!("Rename path template: {s}"),
        Mode::ConfirmRenameUndo(id) => format!("Restore original paths from rename batch {id}? y confirms"),
        Mode::Confirm => "Apply all staged changes? y = confirm, any other key = cancel".into(),
        Mode::ConfirmUndo(id) => format!("Undo batch {id} from verified backups? y = confirm, any other key = cancel"),
        Mode::Help => "m actions (MusicBrainz album lookup) | : palette | C check | Tab panels | v grouping | j/k wrap | Enter open artist/album | Backspace clear search/up tree | Space/b select | x clear | / search | e edit | s suggest | n/w/p/d tabs | a apply | r rescan | c cancel | Esc/q quit".into(),
        Mode::Actions(index) => format!("Actions (j/k, Enter, Esc): {}", ACTIONS[*index].1),
        Mode::Palette(input, index) => format!("Command palette: {input} | {}", matching_actions(input).get(*index).map(|&i| ACTIONS[i].1).unwrap_or("no match")),
        Mode::CheckScope(index) => format!("Check scope (default current folder): {} | j/k, Enter", SCOPES[*index]),
        Mode::ConfirmQuarantine(selected, kept) => format!(
            "Move {} to quarantine; keep {}? y confirms, other key cancels",
            selected.path.display(),
            kept.path.display()
        ),
        Mode::ConfirmRestore(id) => format!("Restore quarantine entry {id}? y confirms"),
        Mode::Results | Mode::DiffReview | Mode::Duplicates | Mode::DuplicateCompare | Mode::QuarantineHistory | Mode::OnlineCandidates | Mode::OnlineMatches | Mode::OnlineReview | Mode::ExportReview | Mode::ConfirmExport | Mode::RenameReview | Mode::ConfirmRename | Mode::History | Mode::ConfirmHistoryUndo(_) => unreachable!(),
    };
    frame.render_widget(
        Paragraph::new(status).block(Block::default().borders(Borders::TOP)),
        vertical[2],
    );
    dialogs::render_palette(frame, area, app);
}
