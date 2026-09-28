use super::*;

pub(super) fn render_results(frame: &mut Frame, area: Rect, app: &App) {
    let issues = app.visible_issues();
    let height = area.height.saturating_sub(2) as usize;
    let start = app.issue_row.saturating_sub(height.saturating_sub(1));
    let items: Vec<_> = issues
        .iter()
        .skip(start)
        .take(height)
        .enumerate()
        .map(|(offset, issue)| {
            let text = format!(
                "{:?} [{}] {}: {}{}",
                issue.confidence,
                issue.rule_id,
                issue.path.display(),
                issue.description,
                if issue.suggestion.is_some() {
                    " [s: stage fix]"
                } else {
                    ""
                }
            );
            ListItem::new(text).style(if start + offset == app.issue_row {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            })
        })
        .collect();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(format!(
                    "Check results: {} ({} visible)",
                    app.issue_filter.label(),
                    issues.len()
                ))
                .borders(Borders::ALL),
        ),
        area,
    );
}
