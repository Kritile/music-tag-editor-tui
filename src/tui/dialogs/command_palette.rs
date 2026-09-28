use super::super::*;

pub(super) fn render(frame: &mut Frame, area: Rect, app: &App) {
    let choices: Option<(String, Vec<(String, bool)>)> = match &app.mode {
        Mode::Actions(index) => Some((
            "Actions".into(),
            ACTIONS
                .iter()
                .enumerate()
                .map(|(i, label)| (label.to_string(), i == *index))
                .collect(),
        )),
        Mode::Palette(input, index) => Some((
            format!("Command palette: {input}"),
            matching_actions(input)
                .iter()
                .enumerate()
                .map(|(row, &i)| (ACTIONS[i].to_string(), row == *index))
                .collect(),
        )),
        Mode::CheckScope(index) => Some((
            "Check scope".into(),
            SCOPES
                .iter()
                .enumerate()
                .map(|(i, label)| (label.to_string(), i == *index))
                .collect(),
        )),
        _ => None,
    };
    if let Some((title, choices)) = choices {
        let popup = Rect {
            x: area.x + area.width / 8,
            y: area.y + 2,
            width: area.width.saturating_mul(3) / 4,
            height: (choices.len() as u16 + 2).min(area.height.saturating_sub(3)),
        };
        let height = popup.height.saturating_sub(2) as usize;
        let selected = choices
            .iter()
            .position(|(_, selected)| *selected)
            .unwrap_or(0);
        let start = selected.saturating_sub(height.saturating_sub(1));
        frame.render_widget(ratatui::widgets::Clear, popup);
        frame.render_widget(
            List::new(
                choices
                    .into_iter()
                    .skip(start)
                    .take(height)
                    .map(|(label, selected)| {
                        ListItem::new(label).style(if selected {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .block(Block::default().title(title).borders(Borders::ALL)),
            popup,
        );
    }
}
