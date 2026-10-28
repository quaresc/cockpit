//! The keys every app shares: moving, quitting, and the verbs more than one app has, each on one key in all of them. An
//! app's own verbs (join, preview, mute…) keep their keys, never one of these; tests/keys.rs checks the apps' actions.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The verbs on the same key in every app that has them, each with the labels its buttons show.
pub const VERBS: [(char, &[&str]); 13] = [
    ('a', &["add"]),
    ('e', &["edit"]),
    ('x', &["done", "reopen", "dismiss"]),
    ('D', &["delete"]),
    ('o', &["open", "go"]),
    ('r', &["run", "rerun", "redo", "review"]),
    ('R', &["refresh"]),
    ('t', &["talk"]),
    ('u', &["undo"]),
    ('v', &["pick", "mark"]),
    ('V', &["pick all alike"]),
    ('w', &["switch view"]),
    ('/', &["filter"]),
];

/// Where a key moves the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// Rows up (negative) or down.
    By(isize),
    /// Pages up (negative) or down.
    Pages(isize),
    Top,
    Bottom,
    /// The previous (negative) or next tab, runbook or group.
    Group(isize),
}

/// The move a key asks for: j/k and the arrows, page up and down, g/G and home/end, [ and ].
pub fn movement(key: &KeyEvent) -> Option<Move> {
    if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return None;
    }
    Some(match key.code {
        KeyCode::Char('j') | KeyCode::Down => Move::By(1),
        KeyCode::Char('k') | KeyCode::Up => Move::By(-1),
        KeyCode::PageDown => Move::Pages(1),
        KeyCode::PageUp => Move::Pages(-1),
        KeyCode::Char('g') | KeyCode::Home => Move::Top,
        KeyCode::Char('G') | KeyCode::End => Move::Bottom,
        KeyCode::Char(']') => Move::Group(1),
        KeyCode::Char('[') => Move::Group(-1),
        _ => return None,
    })
}

/// q, ctrl+c or ctrl+q.
pub fn is_quit(key: &KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('q') => !key.modifiers.contains(KeyModifiers::ALT),
        KeyCode::Char('c') => ctrl,
        _ => false,
    }
}

/// The shared key of a button label, None for an app's own verb.
pub fn key_of(label: &str) -> Option<char> {
    VERBS.iter().find(|(_, labels)| labels.contains(&label)).map(|(k, _)| *k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn the_moves_and_quits() {
        let plain = |c| press(KeyCode::Char(c), KeyModifiers::NONE);
        assert_eq!(movement(&plain('j')), Some(Move::By(1)));
        assert_eq!(movement(&press(KeyCode::Up, KeyModifiers::NONE)), Some(Move::By(-1)));
        assert_eq!(movement(&plain('G')), Some(Move::Bottom));
        assert_eq!(movement(&press(KeyCode::Home, KeyModifiers::NONE)), Some(Move::Top));
        assert_eq!(movement(&plain(']')), Some(Move::Group(1)));
        assert_eq!(movement(&press(KeyCode::Char('g'), KeyModifiers::CONTROL)), None, "a chord is not a move");
        assert!(is_quit(&plain('q')) && is_quit(&press(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(is_quit(&press(KeyCode::Char('q'), KeyModifiers::CONTROL)));
        assert!(!is_quit(&plain('c')));
    }

    #[test]
    fn no_key_holds_two_verbs_nor_a_move() {
        let mut keys: Vec<char> = VERBS.iter().map(|(k, _)| *k).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), VERBS.len());
        for (k, labels) in VERBS {
            assert_eq!(movement(&KeyEvent::new(KeyCode::Char(k), KeyModifiers::NONE)), None, "{labels:?} on a move key");
            assert!(labels.iter().all(|l| key_of(l) == Some(k)), "{labels:?} on two keys");
        }
    }
}
