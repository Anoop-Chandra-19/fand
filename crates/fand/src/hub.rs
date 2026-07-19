//! Shared glue between the control loop and socket connection threads:
//! a latest-status cell (loop → clients) and the command envelope
//! (clients → loop).
//!
//! StatusHub: single writer, many readers. A Condvar wakes subscribers on
//! each publish; the sequence number lets a subscriber tell "new snapshot"
//! from "same one again" after a wakeup.

use std::sync::{mpsc, Condvar, Mutex};
use std::time::Duration;

use fand_proto::{Command, Response, Status};

/// A client request forwarded to the engine thread, plus the channel its
/// answer comes back on (rendezvous style — the connection thread blocks
/// until the engine actually handled it, so replies report real outcomes,
/// not "queued").
pub struct EngineCommand {
    pub cmd: Command,
    pub reply: mpsc::Sender<Response>,
}

#[derive(Default)]
pub struct StatusHub {
    inner: Mutex<Inner>,
    changed: Condvar,
}

#[derive(Default)]
struct Inner {
    seq: u64,
    latest: Option<Status>,
}

impl StatusHub {
    pub fn publish(&self, status: Status) {
        let mut inner = self.inner.lock().unwrap();
        inner.seq += 1;
        inner.latest = Some(status);
        self.changed.notify_all();
    }

    pub fn latest(&self) -> Option<(u64, Status)> {
        let inner = self.inner.lock().unwrap();
        inner.latest.clone().map(|s| (inner.seq, s))
    }

    /// Block until a snapshot newer than `last_seq` is published, or the
    /// timeout passes (returns None — callers just wait again, which keeps
    /// them responsive to their client hanging up).
    pub fn wait_newer(&self, last_seq: u64, timeout: Duration) -> Option<(u64, Status)> {
        let inner = self.inner.lock().unwrap();
        let (inner, result) = self
            .changed
            .wait_timeout_while(inner, timeout, |i| i.seq <= last_seq)
            .unwrap();
        if result.timed_out() {
            None
        } else {
            inner.latest.clone().map(|s| (inner.seq, s))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn status(cpu: f64) -> Status {
        Status {
            temps: BTreeMap::from([("cpu".to_string(), cpu)]),
            channels: BTreeMap::new(),
            config_generation: 0,
            instance: 0,
        }
    }

    #[test]
    fn starts_empty() {
        assert_eq!(StatusHub::default().latest(), None);
    }

    #[test]
    fn publish_bumps_seq() {
        let hub = StatusHub::default();
        hub.publish(status(50.0));
        let (seq1, _) = hub.latest().unwrap();
        hub.publish(status(51.0));
        let (seq2, s) = hub.latest().unwrap();
        assert!(seq2 > seq1);
        assert_eq!(s.temps["cpu"], 51.0);
    }

    #[test]
    fn wait_newer_times_out_without_publish() {
        let hub = StatusHub::default();
        hub.publish(status(50.0));
        let (seq, _) = hub.latest().unwrap();
        assert_eq!(hub.wait_newer(seq, Duration::from_millis(20)), None);
    }

    #[test]
    fn wait_newer_wakes_on_publish() {
        let hub = Arc::new(StatusHub::default());
        let waiter = {
            let hub = Arc::clone(&hub);
            std::thread::spawn(move || hub.wait_newer(0, Duration::from_secs(5)))
        };
        // Give the waiter a moment to park on the condvar first.
        std::thread::sleep(Duration::from_millis(20));
        hub.publish(status(60.0));
        let (seq, s) = waiter.join().unwrap().expect("waiter must see the publish");
        assert_eq!(seq, 1);
        assert_eq!(s.temps["cpu"], 60.0);
    }
}
