use super::*;

pub(super) fn render_online(frame: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.online else {
        return;
    };
    let (title, lines, selected): (String, Vec<String>, usize) = match app.mode {
        Mode::OnlineCandidates => (
            format!("MusicBrainz releases ({})", state.candidates.len()),
            state
                .candidates
                .iter()
                .map(|candidate| {
                    format!(
                        "{} — {} | {} {} | {} tracks",
                        candidate.artist,
                        candidate.title,
                        candidate.date.as_deref().unwrap_or("?"),
                        candidate.country.as_deref().unwrap_or("?"),
                        candidate.tracks
                    )
                })
                .collect(),
            state.candidate_row,
        ),
        Mode::OnlineMatches => {
            let Some(release) = &state.release else {
                return;
            };
            let lines = state
                .local
                .iter()
                .enumerate()
                .map(|(index, track)| {
                    let matched = state
                        .mapping
                        .get(index)
                        .and_then(|item| *item)
                        .and_then(|remote| release.tracks.get(remote));
                    format!(
                        "{} → {}",
                        track.metadata.title.as_deref().unwrap_or("?"),
                        matched.map_or("unmatched".to_string(), |item| format!(
                            "{}-{} {}",
                            item.disc, item.number, item.title
                        ))
                    )
                })
                .collect();
            let remote = release
                .tracks
                .get(state.remote_row)
                .map(|track| {
                    format!(
                        "{}-{} {} — {}",
                        track.disc, track.number, track.title, track.artist
                    )
                })
                .unwrap_or_else(|| "no remote track".into());
            (
                format!("{} — {} | remote: {remote}", release.artist, release.title),
                lines,
                state.local_row,
            )
        }
        Mode::OnlineReview => (
            format!("Review MusicBrainz suggestions ({})", state.proposals.len()),
            state
                .proposals
                .iter()
                .zip(&state.checked)
                .map(|(proposal, selected)| {
                    let track = state
                        .local
                        .iter()
                        .find(|track| track.id == proposal.local_id);
                    format!(
                        "[{}] {} | {}: {:?} → {}",
                        if *selected { 'x' } else { ' ' },
                        track.map_or("?", |track| track
                            .snapshot
                            .path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("?")),
                        proposal.edit.field.label(),
                        proposal.before,
                        proposal.edit.value
                    )
                })
                .collect(),
            state.proposal_row,
        ),
        _ => return,
    };
    let height = area.height.saturating_sub(2) as usize;
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
