use super::*;

mod command_palette;
mod diff;
mod export;
mod rename;

pub(super) fn render_modal(frame: &mut Frame, areas: [Rect; 3], app: &App) -> bool {
    rename::render(frame, areas, app)
        || export::render(frame, areas, app)
        || diff::render(frame, areas, app)
}

pub(super) fn render_palette(frame: &mut Frame, area: Rect, app: &App) {
    command_palette::render(frame, area, app);
}
