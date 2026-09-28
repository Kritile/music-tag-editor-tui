use super::super::*;

pub(super) fn render(frame: &mut Frame, areas: [Rect; 3], app: &App) -> bool {
    if matches!(app.mode, Mode::ExportReview | Mode::ConfirmExport) {
        if let Some(plan) = &app.export_plan {
            let height = areas[1].height.saturating_sub(2) as usize;
            let start = app.preview_row.saturating_sub(height.saturating_sub(1));
            let lines = plan
                .entries
                .iter()
                .skip(start)
                .take(height)
                .enumerate()
                .map(|(offset, entry)| {
                    ListItem::new(format!(
                        "{} {} → {}",
                        if entry.skip { "SKIP" } else { "COPY" },
                        entry.source.display(),
                        entry.destination.display()
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
                        .title(format!(
                            "Export preview: {} files, {} bytes to copy",
                            plan.entries.len(),
                            plan.bytes_to_copy
                        ))
                        .borders(Borders::ALL),
                ),
                areas[1],
            );
            let help = if matches!(app.mode, Mode::ConfirmExport) {
                "Export these copies? y confirms; any other key returns"
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
