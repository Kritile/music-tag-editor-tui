use super::{Action, Mode};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

fn vertical(code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Up | KeyCode::Char('k') => Some(Action::MoveUp),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::MoveDown),
        _ => None,
    }
}

fn list(code: KeyCode) -> Option<Action> {
    vertical(code).or(match code {
        KeyCode::Esc => Some(Action::Back),
        KeyCode::Enter => Some(Action::Open),
        _ => None,
    })
}

fn text_input(code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Esc => Some(Action::Back),
        KeyCode::Enter => Some(Action::Open),
        KeyCode::Backspace => Some(Action::DeleteBackward),
        KeyCode::Char(character) => Some(Action::Input(character)),
        _ => None,
    }
}

fn normal(code: KeyCode) -> Option<Action> {
    Some(match code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Esc => Action::Back,
        KeyCode::Char('?') => Action::Help,
        KeyCode::Up | KeyCode::Char('k') => Action::MoveUp,
        KeyCode::Down | KeyCode::Char('j') => Action::MoveDown,
        KeyCode::Enter => Action::Open,
        KeyCode::Backspace => Action::DeleteBackward,
        KeyCode::Tab => Action::NextFocus,
        KeyCode::Char('v') => Action::CycleGrouping,
        KeyCode::Char('/') => Action::Search,
        KeyCode::Char(' ') => Action::SelectCurrent,
        KeyCode::Char('b') => Action::SelectVisible,
        KeyCode::Char('x') => Action::ClearSelection,
        KeyCode::Char('e') => Action::Edit,
        KeyCode::Char('s') => Action::StageSuggestion,
        KeyCode::Char('d') => Action::ReviewDiff,
        KeyCode::Char('n') => Action::ShowNormalized,
        KeyCode::Char('w') => Action::ShowRaw,
        KeyCode::Char('p') => Action::ShowDevice,
        KeyCode::Char('a') => Action::Apply,
        KeyCode::Char('r') => Action::Refresh,
        KeyCode::Char('C') => Action::Check,
        KeyCode::Char('m') => Action::OpenActions,
        KeyCode::Char(':') => Action::OpenCommandPalette,
        KeyCode::Char('c') => Action::CancelWork,
        _ => return None,
    })
}

pub(super) fn map(key: KeyEvent, mode: &Mode) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    let code = key.code;
    match mode {
        Mode::Normal => normal(code),
        Mode::Actions(_)
        | Mode::CheckScope(_)
        | Mode::QuarantineHistory
        | Mode::OnlineCandidates
        | Mode::ExportReview
        | Mode::RenameReview => list(code),
        Mode::Palette(_, _) => match code {
            KeyCode::Up => Some(Action::MoveUp),
            KeyCode::Down => Some(Action::MoveDown),
            _ => text_input(code),
        },
        Mode::Results => match code {
            KeyCode::Char('q') => Some(Action::Back),
            KeyCode::Char('f') => Some(Action::FilterIssues),
            KeyCode::Char('s') => Some(Action::StageSuggestion),
            _ => list(code),
        },
        Mode::DiffReview => match code {
            KeyCode::Char('a') => Some(Action::Apply),
            _ => list(code),
        },
        Mode::Duplicates => match code {
            KeyCode::Char('q') => Some(Action::Back),
            _ => list(code),
        },
        Mode::DuplicateCompare => match code {
            KeyCode::Char('x') => Some(Action::Quarantine),
            _ => list(code),
        },
        Mode::OnlineMatches => match code {
            KeyCode::Left | KeyCode::Char('h') => Some(Action::MoveLeft),
            KeyCode::Right | KeyCode::Char('l') => Some(Action::MoveRight),
            KeyCode::Char('x') => Some(Action::ClearMapping),
            KeyCode::Char('r') => Some(Action::ReviewMatches),
            _ => list(code),
        },
        Mode::OnlineReview => match code {
            KeyCode::Char(' ') => Some(Action::SelectCurrent),
            KeyCode::Char('s') => Some(Action::StageProposals),
            _ => list(code),
        },
        Mode::Search(_) | Mode::Edit(_) | Mode::ExportPath(_) | Mode::RenameTemplate(_) => {
            text_input(code)
        }
        Mode::Confirm
        | Mode::ConfirmUndo(_)
        | Mode::ConfirmQuarantine(_, _)
        | Mode::ConfirmRestore(_)
        | Mode::ConfirmExport
        | Mode::ConfirmRename
        | Mode::ConfirmRenameUndo(_) => Some(if code == KeyCode::Char('y') {
            Action::Confirm
        } else {
            Action::Dismiss
        }),
        Mode::Help => Some(Action::Back),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn maps_press_and_ignores_release() {
        assert_eq!(
            map(key(KeyCode::Down), &Mode::Normal),
            Some(Action::MoveDown)
        );
        let release =
            KeyEvent::new_with_kind(KeyCode::Down, KeyModifiers::NONE, KeyEventKind::Release);
        assert_eq!(map(release, &Mode::Normal), None);
    }

    #[test]
    fn commands_and_text_depend_on_mode() {
        assert_eq!(
            map(key(KeyCode::Char('a')), &Mode::Normal),
            Some(Action::Apply)
        );
        assert_eq!(
            map(key(KeyCode::Char('a')), &Mode::Palette(String::new(), 0)),
            Some(Action::Input('a'))
        );
        assert_eq!(
            map(key(KeyCode::Enter), &Mode::Actions(0)),
            Some(Action::Open)
        );
        assert_eq!(
            map(key(KeyCode::Char('y')), &Mode::Confirm),
            Some(Action::Confirm)
        );
        assert_eq!(
            map(key(KeyCode::Char('n')), &Mode::Confirm),
            Some(Action::Dismiss)
        );
    }

    #[test]
    fn maps_contextual_list_and_online_commands() {
        assert_eq!(
            map(key(KeyCode::Char('j')), &Mode::Results),
            Some(Action::MoveDown)
        );
        assert_eq!(
            map(key(KeyCode::Char('j')), &Mode::Palette(String::new(), 0)),
            Some(Action::Input('j'))
        );
        assert_eq!(
            map(key(KeyCode::Char('f')), &Mode::Results),
            Some(Action::FilterIssues)
        );
        assert_eq!(
            map(key(KeyCode::Char('x')), &Mode::DuplicateCompare),
            Some(Action::Quarantine)
        );
        assert_eq!(
            map(key(KeyCode::Char('h')), &Mode::OnlineMatches),
            Some(Action::MoveLeft)
        );
        assert_eq!(
            map(key(KeyCode::Char('r')), &Mode::OnlineMatches),
            Some(Action::ReviewMatches)
        );
        assert_eq!(
            map(key(KeyCode::Char('s')), &Mode::OnlineReview),
            Some(Action::StageProposals)
        );
    }
}
