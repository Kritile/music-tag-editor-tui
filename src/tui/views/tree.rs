use super::*;

pub(in crate::tui) fn render_tree(frame: &mut Frame, area: Rect, app: &App) {
    let height = area.height.saturating_sub(2) as usize;
    let start = app.group_index.saturating_sub(height.saturating_sub(1));
    let items: Vec<ListItem> = app
        .groups
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, group)| {
            let style = if index == app.group_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(group.as_str()).style(style)
        })
        .collect();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(format!(
                    "{}{}",
                    app.group_mode.label(),
                    if app.focus == Focus::Tree { " *" } else { "" }
                ))
                .borders(Borders::ALL),
        ),
        area,
    );
}
