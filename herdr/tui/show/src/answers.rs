//! Answers on their way to the session: one `<name>.<block>.answer` file at a time per block, the next written only
//! once the mod has taken the last, so several comments in a row each reach it.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use serde_json::Value;

use cockpit_core::files;

#[derive(Default)]
pub struct Outbox {
    waiting: VecDeque<(PathBuf, Value)>,
}

impl Outbox {
    pub fn send(&mut self, path: PathBuf, answer: Value) {
        self.waiting.push_back((path, answer));
    }

    /// Writes every answer whose file is free, in order; the errors of those that could not be written.
    pub fn flush(&mut self) -> Vec<String> {
        let (mut busy, mut errors) = (HashSet::new(), Vec::new());
        self.waiting.retain(|(path, answer)| {
            if busy.contains(path) || path.exists() {
                busy.insert(path.clone());
                return true;
            }
            busy.insert(path.clone());
            if let Err(err) = files::write(path, answer) {
                errors.push(format!("the answer was not written: {err}"));
            }
            false
        });
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_next_answer_waits_for_the_last_to_be_taken() {
        let dir = std::env::temp_dir().join(format!("cockpit-show-outbox-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v.0.answer");
        let mut outbox = Outbox::default();
        outbox.send(path.clone(), json!({"text": "one"}));
        outbox.send(path.clone(), json!({"text": "two"}));
        outbox.flush();
        assert_eq!(files::load(&path), Some(json!({"text": "one"})));
        outbox.flush();
        assert_eq!(files::load(&path), Some(json!({"text": "one"})));
        std::fs::remove_file(&path).unwrap();
        outbox.flush();
        assert_eq!(files::load(&path), Some(json!({"text": "two"})));
        assert!(outbox.waiting.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
