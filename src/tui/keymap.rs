use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Quit,
    Help,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Activate,
    Cancel,
    DeleteBackward,
    NextFocus,
    CycleGrouping,
    Search,
    SelectCurrent,
    SelectVisible,
    ClearSelection,
    Edit,
    StageSuggestion,
    ReviewDiff,
    ShowNormalized,
    ShowRaw,
    ShowDevice,
    Apply,
    Refresh,
    Check,
    OpenActions,
    OpenPalette,
    CancelWork,
    Character(char),
    Other,
}

pub(super) fn map(key: KeyEvent, normal_mode: bool) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    if normal_mode {
        let command = match key.code {
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Char('?') => Some(Action::Help),
            KeyCode::Char('j') => Some(Action::MoveDown),
            KeyCode::Char('k') => Some(Action::MoveUp),
            KeyCode::Char('v') => Some(Action::CycleGrouping),
            KeyCode::Char('/') => Some(Action::Search),
            KeyCode::Char(' ') => Some(Action::SelectCurrent),
            KeyCode::Char('b') => Some(Action::SelectVisible),
            KeyCode::Char('x') => Some(Action::ClearSelection),
            KeyCode::Char('e') => Some(Action::Edit),
            KeyCode::Char('s') => Some(Action::StageSuggestion),
            KeyCode::Char('d') => Some(Action::ReviewDiff),
            KeyCode::Char('n') => Some(Action::ShowNormalized),
            KeyCode::Char('w') => Some(Action::ShowRaw),
            KeyCode::Char('p') => Some(Action::ShowDevice),
            KeyCode::Char('a') => Some(Action::Apply),
            KeyCode::Char('r') => Some(Action::Refresh),
            KeyCode::Char('C') => Some(Action::Check),
            KeyCode::Char('m') => Some(Action::OpenActions),
            KeyCode::Char(':') => Some(Action::OpenPalette),
            KeyCode::Char('c') => Some(Action::CancelWork),
            _ => None,
        };
        if command.is_some() {
            return command;
        }
    }
    Some(match key.code {
        KeyCode::Up => Action::MoveUp,
        KeyCode::Down => Action::MoveDown,
        KeyCode::Left => Action::MoveLeft,
        KeyCode::Right => Action::MoveRight,
        KeyCode::Enter => Action::Activate,
        KeyCode::Esc => Action::Cancel,
        KeyCode::Backspace => Action::DeleteBackward,
        KeyCode::Tab => Action::NextFocus,
        KeyCode::Char(character) => Action::Character(character),
        _ => Action::Other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn maps_press_to_action_and_ignores_release() {
        let press = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(map(press, true), Some(Action::MoveDown));

        let release =
            KeyEvent::new_with_kind(KeyCode::Down, KeyModifiers::NONE, KeyEventKind::Release);
        assert_eq!(map(release, true), None);
    }

    #[test]
    fn normal_shortcuts_do_not_replace_dialog_text() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(map(key, true), Some(Action::Apply));
        assert_eq!(map(key, false), Some(Action::Character('a')));
    }
}
