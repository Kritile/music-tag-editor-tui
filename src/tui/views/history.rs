use super::*;
use ratatui::widgets::ListState;

pub(super) fn render_history(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Min(1)])
        .split(area);
    let items: Vec<ListItem> = app
        .history_entries
        .iter()
        .map(|entry| {
            ListItem::new(format!(
                "{}  {}  {:?}  {:?}",
                entry.time_utc(),
                entry.summary(),
                entry.status,
                entry.verification
            ))
        })
        .collect();
    let mut selection = ListState::default();
    if !items.is_empty() {
        selection.select(Some(app.history_row.min(items.len() - 1)));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().title("History").borders(Borders::ALL))
            .highlight_style(Style::default().fg(Color::Black).bg(Color::White)),
        rows[0],
        &mut selection,
    );
    let detail = app.history_entries.get(app.history_row).map_or_else(
        || "No journaled operations".to_string(),
        |entry| {
            let mut lines = vec![
                format!(
                    "Type: {:?}  Time: {}  ID: {}",
                    entry.key.kind,
                    entry.time_utc(),
                    entry.key.id
                ),
                format!(
                    "Files: {}  Status: {:?}  Verification: {:?}",
                    entry.files.len(),
                    entry.status,
                    entry.verification
                ),
                format!(
                    "Reversible: {}  Safe to undo now: {}",
                    entry.reversible, entry.undo_safe
                ),
            ];
            for file in &entry.files {
                lines.push(format!("{} [{:?}]", file.path.display(), file.verification));
                lines.push(format!("  Before: {}", file.before));
                lines.push(format!("  After:  {}", file.after));
            }
            lines.join("\n")
        },
    );
    frame.render_widget(
        Paragraph::new(detail)
            .block(
                Block::default()
                    .title("Operation details")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false })
            .scroll((app.history_detail_scroll, 0)),
        rows[1],
    );
}
