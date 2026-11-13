//! A subscriber's lines on their way to its connection. The queue holds at most a budget of bytes: a subscriber that
//! falls that far behind is cut off and its connection closes, so the versions it does not read never pile up in memory.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

/// How far behind a subscriber may fall, in bytes of lines not yet written to its connection.
pub const BUDGET: usize = 16 << 20;

struct Backlog {
    budget: usize,
    bytes: AtomicUsize,
    cut: AtomicBool,
}

/// The sending side, cloned for each part of the hub that pushes to the subscriber.
#[derive(Clone)]
pub struct Outbox {
    tx: Sender<String>,
    backlog: Arc<Backlog>,
}

/// The receiving side, which the connection's writer reads.
pub struct Lines {
    rx: Receiver<String>,
    backlog: Arc<Backlog>,
}

/// The line was not queued: the subscriber's connection is gone, or it fell behind and was cut off.
#[derive(Debug, PartialEq)]
pub struct Gone;

/// A subscriber's queue, holding at most BUDGET bytes.
pub fn channel() -> (Outbox, Lines) {
    with_budget(BUDGET)
}

fn with_budget(budget: usize) -> (Outbox, Lines) {
    let (tx, rx) = mpsc::channel();
    let backlog = Arc::new(Backlog { budget, bytes: AtomicUsize::new(0), cut: AtomicBool::new(false) });
    (Outbox { tx, backlog: backlog.clone() }, Lines { rx, backlog })
}

impl Outbox {
    /// Queues the line; a line past the budget cuts the subscriber off, and every send after that fails.
    pub fn send(&self, line: String) -> Result<(), Gone> {
        let backlog = &self.backlog;
        if backlog.cut.load(Ordering::Acquire) {
            return Err(Gone);
        }
        if backlog.bytes.fetch_add(line.len(), Ordering::AcqRel) + line.len() > backlog.budget {
            backlog.cut.store(true, Ordering::Release);
            return Err(Gone);
        }
        self.tx.send(line).map_err(|_| Gone)
    }
}

impl Lines {
    /// The next line, waiting at most `timeout`; Disconnected once the subscriber is cut off or no sender is left.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<String, RecvTimeoutError> {
        if self.backlog.cut.load(Ordering::Acquire) {
            return Err(RecvTimeoutError::Disconnected);
        }
        let line = self.rx.recv_timeout(timeout)?;
        self.backlog.bytes.fetch_sub(line.len(), Ordering::AcqRel);
        Ok(line)
    }

    /// The lines queued now, without waiting.
    pub fn try_iter(&self) -> impl Iterator<Item = String> + '_ {
        std::iter::from_fn(|| self.recv_timeout(Duration::ZERO).ok())
    }

    /// The lines as they come, until the subscriber is cut off or no sender is left.
    pub fn iter(&self) -> impl Iterator<Item = String> + '_ {
        std::iter::from_fn(|| self.recv_timeout(Duration::MAX).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_read_free_the_budget_for_more() {
        let (tx, rx) = with_budget(10);
        for _ in 0..5 {
            assert_eq!(tx.send("12345678".into()), Ok(()));
            assert_eq!(rx.try_iter().collect::<Vec<_>>(), ["12345678"]);
        }
    }

    #[test]
    fn a_subscriber_past_its_budget_is_cut_off_for_every_sender() {
        let (tx, rx) = with_budget(10);
        let other = tx.clone();
        assert_eq!(tx.send("123456".into()), Ok(()));
        assert_eq!(tx.send("123456".into()), Err(Gone));
        assert_eq!(other.send("1".into()), Err(Gone), "a small line after the cut is refused too");
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)), Err(RecvTimeoutError::Disconnected), "the writer stops at once");
    }

    #[test]
    fn a_receiver_gone_fails_the_send() {
        let (tx, rx) = channel();
        drop(rx);
        assert_eq!(tx.send("x".into()), Err(Gone));
    }
}
