use super::super::*;

pub(super) fn render(frame: &mut Frame, areas: [Rect; 3], app: &App) -> bool {
    if matches!(app.mode, Mode::DiffReview) {
        let height = areas[1].height.saturating_sub(2) as usize;
        let start = app.review_row.saturating_sub(height.saturating_sub(1));
        let lines = app
            .review_lines
            .iter()
            .skip(start)
            .take(height)
            .enumerate()
            .map(|(offset, line)| {
                ListItem::new(line.as_str()).style(if start + offset == app.review_row {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                })
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            List::new(lines).block(
                Block::default()
                    .title("Per-file diff")
                    .borders(Borders::ALL),
            ),
            areas[1],
        );
        frame.render_widget(
            Paragraph::new("Review each file | j/k scroll | a apply | Esc back")
                .block(Block::default().borders(Borders::TOP)),
            areas[2],
        );
        return true;
    }
    false
}
