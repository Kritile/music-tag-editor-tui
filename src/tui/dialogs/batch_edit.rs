use super::super::*;
use crate::tui::batch_edit::{Change, FIELDS};

pub(super) fn render(frame: &mut Frame, areas: [Rect; 3], app: &App) -> bool {
    if !matches!(app.mode, Mode::BatchEdit | Mode::BatchEditInput(_)) {
        return false;
    }
    let Some(editor) = app.batch_editor.as_ref() else {
        return false;
    };
    let height = areas[1].height.saturating_sub(2) as usize;
    let start = editor.row.saturating_sub(height.saturating_sub(1));
    let rows: Vec<ListItem> = FIELDS
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, field)| {
            let original = if editor.mixed[index] {
                "<mixed>".to_string()
            } else {
                editor.original[index]
                    .clone()
                    .unwrap_or_else(|| "<empty>".into())
            };
            let change = match &editor.changes[index] {
                Change::Unchanged => "unchanged".into(),
                Change::Set(value) => format!("set: {value}"),
                Change::Clear => "clear".into(),
            };
            ListItem::new(format!("{}: {} → {}", field.label(), original, change)).style(
                if index == editor.row {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                },
            )
        })
        .collect();
    frame.render_widget(
        List::new(rows).block(
            Block::default()
                .title(format!("Edit selected ({} tracks)", editor.tracks.len()))
                .borders(Borders::ALL),
        ),
        areas[1],
    );
    let help = match &app.mode {
        Mode::BatchEditInput(value) => format!(
            "Set {}: {value} | Enter accept | Esc cancel",
            FIELDS[editor.row].label()
        ),
        _ => format!(
            "j/k field | s set | c clear | u unchanged | Enter stage | Esc cancel | {}",
            app.status
        ),
    };
    frame.render_widget(
        Paragraph::new(help).block(Block::default().borders(Borders::TOP)),
        areas[2],
    );
    true
}
