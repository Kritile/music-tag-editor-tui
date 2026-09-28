use super::*;

impl App {
    pub(super) fn key(&mut self, key: KeyEvent, sender: &Sender<Message>) -> bool {
        match keymap::map(key, &self.mode) {
            Some(action) => self.update(action, sender),
            None => false,
        }
    }
}
