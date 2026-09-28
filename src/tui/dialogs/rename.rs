use super::super::*;

pub(super) fn render(frame: &mut Frame, areas: [Rect; 3], app: &App) -> bool {
    if matches!(app.mode, Mode::RenameReview | Mode::ConfirmRename) {
        if let Some(plan) = &app.rename_plan {
            let height = areas[1].height.saturating_sub(2) as usize;
            let start = app.preview_row.saturating_sub(height.saturating_sub(1));
            let lines = plan
                .moves
                .iter()
                .skip(start)
                .take(height)
                .enumerate()
                .map(|(offset, operation)| {
                    ListItem::new(format!(
                        "{} → {}",
                        operation.source.display(),
                        operation.destination.display()
                    ))
                    .style(if start + offset == app.preview_row {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default()
                    })
                })
                .collect::<Vec<_>>();
            frame.render_widget(
                List::new(lines).block(
                    Block::default()
                        .title(format!("Rename preview: {} files", plan.moves.len()))
                        .borders(Borders::ALL),
                ),
                areas[1],
            );
            let help = if matches!(app.mode, Mode::ConfirmRename) {
                "Move these files? y confirms; any other key returns"
            } else {
                "j/k scroll | Enter confirm | Esc back"
            };
            frame.render_widget(
                Paragraph::new(help).block(Block::default().borders(Borders::TOP)),
                areas[2],
            );
        }
        return true;
    }
    false
}
