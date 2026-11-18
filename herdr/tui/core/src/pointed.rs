//! A row a focus request brought into view (`cockpit focus`, a notification, a palette jump): any process can ask for one,
//! so the keys that act on that row wait until the person moves to it or clicks.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keys;

/// What an app says to a key that waits for the person's move.
pub const MOVE_FIRST: &str = "Move to it first (j/k), or click it";

/// Whether the selected row was pointed at from outside rather than moved to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pointed(bool);

impl Pointed {
    /// The row a focus request selected: its keys wait.
    pub fn point(&mut self) {
        self.0 = true;
    }

    /// A click, or the person's own pick: the row is theirs.
    pub fn clear(&mut self) {
        self.0 = false;
    }

    pub fn is_on(self) -> bool {
        self.0
    }

    /// Whether this key waits: true for one that would act on a row pointed at. A move makes the row the person's, and the
    /// keys that only look (quit, the palette, esc, tab, space, filter, help, refresh, switch view, sign in) go on.
    pub fn holds(&mut self, key: &KeyEvent) -> bool {
        if !self.0 {
            return false;
        }
        if keys::movement(key).is_some() {
            self.0 = false;
            return false;
        }
        !looks_only(key)
    }
}

fn looks_only(key: &KeyEvent) -> bool {
    if keys::is_quit(key) {
        return true;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('p') => ctrl,
        KeyCode::Char(' ' | '/' | '?' | 'w' | 'R' | 'L') => !ctrl && !key.modifiers.contains(KeyModifiers::ALT),
        KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn a_row_pointed_at_takes_no_key_that_acts_until_a_move() {
        let mut p = Pointed::default();
        assert!(!p.holds(&press(KeyCode::Enter)), "nothing pointed at: every key goes on");
        p.point();
        for code in [KeyCode::Enter, KeyCode::Char('m'), KeyCode::Char('r'), KeyCode::Char('D'), KeyCode::Char('1')] {
            assert!(p.holds(&press(code)), "{code:?} waits");
        }
        assert!(p.holds(&KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)), "a chord that acts waits too");
        for code in [KeyCode::Esc, KeyCode::Tab, KeyCode::Char('/'), KeyCode::Char('?'), KeyCode::Char('R'), KeyCode::Char('q')] {
            assert!(!p.holds(&press(code)), "{code:?} only looks");
        }
        assert!(!p.holds(&KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)), "the palette opens");
        assert!(p.is_on(), "looking leaves it pointed at");
        assert!(!p.holds(&press(KeyCode::Char('j'))));
        assert!(!p.is_on(), "a move makes it the person's");
        assert!(!p.holds(&press(KeyCode::Enter)));
    }
}
