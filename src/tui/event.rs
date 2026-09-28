use super::*;

impl App {
    pub(super) fn key(&mut self, key: KeyEvent, sender: &Sender<Message>) -> bool {
        match keymap::map(key, matches!(self.mode, Mode::Normal)) {
            Some(action) => self.update(action, sender),
            None => false,
        }
    }
}
