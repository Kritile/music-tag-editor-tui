use super::*;

pub(super) fn render_tracks(frame: &mut Frame, area: Rect, app: &App) {
    let height = area.height.saturating_sub(2) as usize;
    let start = app.row.saturating_sub(height.saturating_sub(1));
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .skip(start)
        .take(height)
        .enumerate()
        .map(|(offset, &index)| {
            let track = &app.tracks[index];
            let marker = if app.selected.contains(&track.id) {
                "[x]"
            } else {
                "[ ]"
            };
            let issue = if app.issues.iter().any(|i| i.track_id == track.id) {
                "!"
            } else {
                " "
            };
            let label = format!(
                "{marker}{issue} {} — {}",
                track.metadata.artist.as_deref().unwrap_or("?"),
                track.metadata.title.as_deref().unwrap_or_else(|| track
                    .snapshot
                    .path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?"))
            );
            let style = if start + offset == app.row {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(label).style(style)
        })
        .collect();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(format!(
                    "Tracks {}{}",
                    app.visible.len(),
                    if app.focus == Focus::Tracks { " *" } else { "" }
                ))
                .borders(Borders::ALL),
        ),
        area,
    );
}
