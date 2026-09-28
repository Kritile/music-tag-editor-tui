use super::*;

pub(super) fn render_inspector(frame: &mut Frame, area: Rect, app: &App) {
    let title = match app.tab {
        Tab::Normalized => "Normalized (n)",
        Tab::Raw => "Raw (w)",
        Tab::Device => "Echo Mini hypothesis (p)",
        Tab::Diff => "Diff (d)",
    };
    let mut lines = Vec::new();
    if let Some(track) = app.current() {
        lines.push(track.snapshot.path.display().to_string());
        match app.tab {
            Tab::Normalized => {
                let m = &track.metadata;
                lines.extend([
                    format!("TITLE: {:?}", m.title),
                    format!("ARTIST: {:?}", m.artist),
                    format!("ARTISTS: {:?}", m.artists),
                    format!("ALBUMARTIST: {:?}", m.album_artist),
                    format!("ALBUM: {:?}", m.album),
                    format!("TRACK: {:?}", m.track),
                    format!("DISC: {:?}", m.disc),
                    format!("DATE: {:?}", m.date),
                    format!("GENRES: {:?}", m.genres),
                    format!("ARTWORK: {}", m.artwork_count),
                    format!("WRITE: {}", track.write_reason),
                ]);
                lines.extend(track.diagnostics.iter().map(|d| format!("CONFLICT: {d}")));
            }
            Tab::Raw => {
                for raw in &track.raw {
                    let value = match &raw.value {
                        RawValue::Text(s) | RawValue::Locator(s) => s.clone(),
                        RawValue::Binary { bytes, sha256 } => {
                            format!("<binary {bytes} bytes sha256 {sha256}>")
                        }
                    };
                    lines.push(format!(
                        "{}:{} #{} {} = {}",
                        raw.container, raw.native_key, raw.order, raw.detail, value
                    ));
                }
            }
            Tab::Device => {
                let artist = track
                    .metadata
                    .album_artist
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .or(track.metadata.artist.as_deref())
                    .unwrap_or("<unknown>");
                let album = track.metadata.album.as_deref().unwrap_or("<unknown>");
                let projected = rules::echo_projection_key(track);
                let count = app
                    .tracks
                    .iter()
                    .filter(|other| rules::echo_projection_key(other) == projected)
                    .count();
                lines.push(format!("Predicted artist: {artist}"));
                lines.push(format!("Predicted album: {album} ({count} tracks)"));
                lines.push("Confidence: Experimental (tag-derived hypothesis)".into());
                lines.push(format!("Model grouping key: {}", projected));
                lines.push(
                    "Hypothesis only: firmware may group differently or cache prior scans.".into(),
                );
                for issue in app.issues.iter().filter(|i| i.track_id == track.id) {
                    lines.push(format!(
                        "[{:?}] {}: {}",
                        issue.confidence, issue.rule_id, issue.description
                    ));
                }
            }
            Tab::Diff => {
                let (files, values, backup_bytes) = changes::summary(&app.staged);
                lines.push(format!("Batch: {files} files, {values} values, 0 deletions; backup ≥ {backup_bytes} bytes"));
                for pending in app
                    .staged
                    .iter()
                    .filter(|p| p.expected.path == track.snapshot.path)
                {
                    lines.extend(changes::diff(pending));
                }
                if lines.len() == 2 {
                    lines.push("No changes staged for this file".into());
                }
            }
        }
    } else {
        lines.push("No track selected".into());
    }
    frame.render_widget(
        Paragraph::new(lines.join("\n"))
            .scroll((app.inspector_scroll, 0))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(format!(
                        "{title}{}",
                        if app.focus == Focus::Inspector {
                            " *"
                        } else {
                            ""
                        }
                    ))
                    .borders(Borders::ALL),
            ),
        area,
    );
}
