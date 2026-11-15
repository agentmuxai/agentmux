// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Health of the messaging path, for `/health` and `app.health`.
//!
//! On 2026-11-15 narko's message handler stopped answering for two hours while `/health` kept saying `ok`, because it only said
//! that the process answered. Now the report also says whether the handler's lock can be taken (every message, spawn and Kanban
//! load needs it), how many times a panic left it poisoned and it was recovered, and whether srv is panicking in bulk
//! (`backend::panic_watch`). Either problem makes the status `degraded`.
//!
//! The HTTP status stays 200: `agentmux`'s own `health_ok` (agentmux_cli/process.rs) reads anything else as "no server here" and
//! could start a second one beside a working srv. The word is in the body.

use std::time::Duration;

use serde_json::{json, Value};

use crate::backend::panic_watch::{self, SrvPanicsEvent};
use crate::backend::reactive::ReactiveHandler;

/// How long the lock may stay taken before the path counts as stuck. A delivery holds it for milliseconds.
const LOCK_WAIT: Duration = Duration::from_millis(500);
const LOCK_POLL: Duration = Duration::from_millis(10);

/// What the messaging path looks like just now.
pub(crate) struct MessagingHealth {
    lock_free: bool,
    recovered_lock: u64,
    panics: SrvPanicsEvent,
}

impl MessagingHealth {
    pub(crate) fn new(lock_free: bool, recovered_lock: u64, panics: SrvPanicsEvent) -> Self {
        Self { lock_free, recovered_lock, panics }
    }

    /// Looks now. The wait for the lock is bounded and does not block the executor.
    pub(crate) async fn probe(handler: &ReactiveHandler) -> Self {
        let lock_free = lock_taken_within(|| handler.lock_is_free(), LOCK_WAIT, LOCK_POLL).await;
        Self::new(lock_free, handler.recovered_lock_count(), panic_watch::snapshot())
    }

    /// `ok`, or `degraded`: the lock stayed taken, or srv is panicking above the warning level.
    pub(crate) fn status(&self) -> &'static str {
        if self.lock_free && self.panics.level == "normal" {
            "ok"
        } else {
            "degraded"
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "status": self.status(),
            "handler_lock": if self.lock_free { "free" } else { "busy" },
            "recovered_lock": self.recovered_lock,
            "panics": {
                "level": self.panics.level,
                "last_minute": self.panics.last_minute,
                "total": self.panics.total,
                "top": self.panics.top.iter().map(|p| json!({ "location": p.location, "count": p.count })).collect::<Vec<_>>(),
            },
        })
    }
}

/// Whether `free` answers true within `wait`, asking every `poll`.
async fn lock_taken_within(free: impl Fn() -> bool, wait: Duration, poll: Duration) -> bool {
    let mut waited = Duration::ZERO;
    loop {
        if free() {
            return true;
        }
        if waited >= wait {
            return false;
        }
        tokio::time::sleep(poll).await;
        waited += poll;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn panics(level: &str, last_minute: u32) -> SrvPanicsEvent {
        SrvPanicsEvent {
            kind: "panics".into(),
            level: level.into(),
            since_ms: None,
            last_minute,
            total: 7,
            recovered_lock: 0,
            top: vec![panic_watch::PanicPlace { location: "backend/x.rs:1".into(), count: 7 }],
        }
    }

    #[test]
    fn a_free_lock_and_a_quiet_srv_are_ok() {
        let h = MessagingHealth::new(true, 0, panics("normal", 0));
        assert_eq!(h.status(), "ok");
        let json = h.to_json();
        assert_eq!((json["status"].as_str(), json["handler_lock"].as_str(), json["recovered_lock"].as_u64()), (Some("ok"), Some("free"), Some(0)));
        assert_eq!(json["panics"]["top"][0]["location"], "backend/x.rs:1");
    }

    #[test]
    fn a_stuck_lock_is_degraded() {
        let h = MessagingHealth::new(false, 0, panics("normal", 0));
        assert_eq!(h.status(), "degraded");
        assert_eq!(h.to_json()["handler_lock"], "busy");
    }

    #[test]
    fn a_panic_storm_is_degraded_even_with_the_lock_free() {
        for level in ["warn", "critical"] {
            assert_eq!(MessagingHealth::new(true, 0, panics(level, 500)).status(), "degraded", "{level}");
        }
    }

    #[test]
    fn a_recovered_poisoning_alone_is_reported_but_is_not_degraded() {
        let h = MessagingHealth::new(true, 3, panics("normal", 1));
        assert_eq!(h.status(), "ok", "messaging works again: the count is history");
        assert_eq!(h.to_json()["recovered_lock"], 3);
    }

    #[tokio::test]
    async fn the_wait_ends_when_the_lock_frees_and_gives_up_when_it_does_not() {
        let asked = AtomicU32::new(0);
        let frees_on_third = || asked.fetch_add(1, Ordering::SeqCst) >= 2;
        assert!(lock_taken_within(frees_on_third, Duration::from_millis(200), Duration::from_millis(1)).await);
        assert_eq!(asked.load(Ordering::SeqCst), 3);
        let started = std::time::Instant::now();
        assert!(!lock_taken_within(|| false, Duration::from_millis(30), Duration::from_millis(5)).await, "a lock held for good is not free");
        assert!(started.elapsed() < Duration::from_secs(2), "and the wait is bounded");
    }

    #[tokio::test]
    async fn the_probe_sees_a_held_handler_lock_and_a_free_one() {
        let handler = ReactiveHandler::new();
        assert_eq!(MessagingHealth::probe(&handler).await.to_json()["handler_lock"], "free");
        // Held on a thread of its own, so the probe's wait runs out against a real held lock.
        let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let handler: &'static ReactiveHandler = Box::leak(Box::new(handler));
        let holder = std::thread::spawn(move || {
            let _guard = handler.hold_lock_for_test();
            held_tx.send(()).unwrap();
            let _ = done_rx.recv();
        });
        held_rx.recv().unwrap();
        let busy = MessagingHealth::probe(handler).await;
        assert_eq!((busy.to_json()["handler_lock"].as_str(), busy.status()), (Some("busy"), "degraded"));
        done_tx.send(()).unwrap();
        holder.join().unwrap();
        assert_eq!(MessagingHealth::probe(handler).await.status(), "ok", "free again");
    }
}
