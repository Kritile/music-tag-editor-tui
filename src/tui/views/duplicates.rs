use super::*;

pub(super) fn render_duplicate_screen(frame: &mut Frame, area: Rect, app: &App) {
    if matches!(
        app.mode,
        Mode::DuplicateCompare | Mode::ConfirmQuarantine(_, _)
    ) {
        let Some(group) = app
            .duplicate_report
            .as_ref()
            .and_then(|r| r.groups.get(app.duplicate_group))
        else {
            frame.render_widget(Paragraph::new("No duplicate group selected"), area);
            return;
        };
        let Some(selected) = group.members.get(app.duplicate_member) else {
            return;
        };
        let Some(kept) = group
            .members
            .get(if app.duplicate_member == 0 { 1 } else { 0 })
        else {
            return;
        };
        let describe = |label: &str, member: &crate::duplicates::DuplicateMember| {
            format!(
                "{label}: {}\n  {} — {}\n  Album: {} | disc {:?} track {:?}\n  {} | {} bytes\n  SHA-256: {}",
                member.path.display(),
                member.artist.as_deref().unwrap_or("?"),
                member.title.as_deref().unwrap_or("?"),
                member.album.as_deref().unwrap_or("?"),
                member.disc,
                member.number,
                member.format,
                member.size,
                member.sha256.as_deref().unwrap_or("unavailable")
            )
        };
        frame.render_widget(
            Paragraph::new(format!(
                "{}\n\n{}",
                describe("MOVE", selected),
                describe("KEEP", kept)
            ))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(format!("Compare: {:?}", group.kind))
                    .borders(Borders::ALL),
            ),
            area,
        );
        return;
    }
    let height = area.height.saturating_sub(2) as usize;
    let (title, lines, selected): (String, Vec<String>, usize) = match app.mode {
        Mode::Duplicates => {
            let groups = app
                .duplicate_report
                .as_ref()
                .map(|r| r.groups.as_slice())
                .unwrap_or(&[]);
            let lines = groups
                .iter()
                .map(|group| {
                    format!(
                        "{:?}: {} files | {} — {}",
                        group.kind,
                        group.members.len(),
                        group.members[0].artist.as_deref().unwrap_or("?"),
                        group.members[0].title.as_deref().unwrap_or("?")
                    )
                })
                .collect();
            (
                format!("Duplicate groups ({})", groups.len()),
                lines,
                app.duplicate_group,
            )
        }
        Mode::QuarantineHistory | Mode::ConfirmRestore(_) => {
            let lines = app
                .quarantine_records
                .iter()
                .map(|record| {
                    format!(
                        "{:?} {} | {} | kept {}",
                        record.status,
                        record.id,
                        record.original.display(),
                        record.peer.display()
                    )
                })
                .collect();
            ("Quarantine history".into(), lines, app.quarantine_row)
        }
        _ => return,
    };
    let start = selected.saturating_sub(height.saturating_sub(1));
    let items = lines
        .into_iter()
        .skip(start)
        .take(height)
        .enumerate()
        .map(|(offset, line)| {
            ListItem::new(line).style(if start + offset == selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            })
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}
