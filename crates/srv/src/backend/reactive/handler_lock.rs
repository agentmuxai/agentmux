// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The handler's lock, which a panic cannot leave closed.
//!
//! Every `ReactiveHandler` method takes this lock, and a delivery calls code the handler does not own (the controller's
//! message sender, the identity confirmers) while it holds it. A panic in any of them used to poison a plain
//! `std::sync::Mutex`, and every later `lock().unwrap()` then panicked too: on 2026-11-15 one panic on the build-slot
//! idle watch stopped all agent messaging, spawns and the Kanban on a machine for two hours, until AgentMux was
//! restarted (docs/incident/RCA_BUILD_SLOT_IDLE_NOTE_POISONS_MESSAGE_HANDLER_2026_11_15.md).
//!
//! [`HandlerLock::lock`] takes the lock whatever state it is in. After a panic it clears the poison, resets what a
//! half-finished delivery leaves in the handler, and logs one warning. A panic costs the delivery it happened in, not
//! the ones after it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};

use super::handler::{Handler, ReactiveHandler};

/// `Mutex<Handler>` whose locks recover from a poisoning panic instead of passing it on.
pub(super) struct HandlerLock {
    mutex: Mutex<Handler>,
    /// How many times a poisoned lock was recovered since srv started: `/health` and the status bar report it.
    recovered: AtomicU64,
}

impl HandlerLock {
    pub(super) fn new(handler: Handler) -> Self {
        Self { mutex: Mutex::new(handler), recovered: AtomicU64::new(0) }
    }

    /// Waits for the lock. Never panics because another thread did.
    pub(super) fn lock(&self) -> MutexGuard<'_, Handler> {
        self.mutex.lock().unwrap_or_else(|poisoned| self.recover(poisoned))
    }

    /// Takes the lock if it is free right now; `None` when another thread holds it. A poisoned lock is recovered, not refused.
    pub(super) fn try_lock(&self) -> Option<MutexGuard<'_, Handler>> {
        match self.mutex.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Poisoned(poisoned)) => Some(self.recover(poisoned)),
        }
    }

    #[cold]
    fn recover<'a>(&'a self, poisoned: PoisonError<MutexGuard<'a, Handler>>) -> MutexGuard<'a, Handler> {
        let mut handler = poisoned.into_inner();
        handler.reset_delivery_context();
        self.recovered.fetch_add(1, Ordering::Relaxed);
        // Cleared while the guard is held: the next thread to lock sees a healthy lock, and the warning is written once per panic.
        self.mutex.clear_poison();
        tracing::warn!("reactive: a panic under the handler lock was recovered; the delivery it happened in is lost, messaging goes on");
        handler
    }

    pub(super) fn recovered(&self) -> u64 {
        self.recovered.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn is_poisoned(&self) -> bool {
        self.mutex.is_poisoned()
    }
}

impl ReactiveHandler {
    /// How many times the handler's lock was found poisoned and recovered since srv started.
    pub fn recovered_lock_count(&self) -> u64 {
        self.inner.recovered()
    }

    /// Holds the lock until the guard is dropped: for a test of what health says about a lock that stays taken.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hold_lock_for_test(&self) -> std::sync::MutexGuard<'_, Handler> {
        self.inner.lock()
    }

    /// Whether the lock could be taken just now (and is let go at once): the pulse of the messaging path, for health. A lock held for
    /// good, as a deadlock or a stuck delivery would, is never free.
    pub fn lock_is_free(&self) -> bool {
        self.inner.try_lock().is_some()
    }
}

impl Handler {
    /// `inject_message_inner` puts the sender's identity, the WAN verdict and the trust label of the delivery in flight
    /// into the handler and restores them when the delivery ends. A panic in between skips the restore, and the next
    /// audit entry would carry another delivery's sender. Nothing is in flight when the lock is recovered (the one
    /// thread that held it is gone), so the idle values are the right ones.
    pub(super) fn reset_delivery_context(&mut self) {
        self.delivery_source_uid = String::new();
        self.delivery_wan = None;
        self.delivery_lan_claimed_uid = String::new();
        self.delivery_turn = None;
        self.delivery_how = None;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::super::handler::ReactiveHandler;
    use super::super::types::{InjectionRequest, SenderDelivery};

    /// Runs `f` on a thread of its own, which is expected to panic, and returns whether it did.
    fn panics_on_a_thread(f: impl FnOnce() + Send) -> bool {
        std::thread::scope(|s| s.spawn(f).join().is_err())
    }

    /// A message sender that panics on its first call (as the controller did without a Tokio runtime) and delivers after that.
    fn sender_that_panics_once() -> super::super::types::MessageSender {
        let first = AtomicBool::new(true);
        Arc::new(move |_block: &str, _text: &str, _inline: &[serde_json::Value], _origin: &crate::backend::message_origin::MessageOrigin, _expires: Option<i64>| {
            if first.swap(false, Ordering::SeqCst) {
                panic!("there is no reactor running, must be called from the context of a Tokio 1.x runtime");
            }
            Ok(SenderDelivery::Delivered)
        })
    }

    #[test]
    fn a_panic_under_the_lock_does_not_stop_the_next_caller() {
        let handler = ReactiveHandler::new();
        handler.inner.lock().register_agent("agent1", "block1", None).unwrap();
        assert!(panics_on_a_thread(|| {
            let mut guard = handler.inner.lock();
            guard.delivery_source_uid = "another-delivery".to_string();
            panic!("a panic while the lock is held");
        }));
        assert!(handler.inner.is_poisoned(), "the std lock was poisoned: this is the state that used to stop everything");
        assert_eq!(handler.list_agents().len(), 1, "the next caller gets the lock and its data");
        assert!(!handler.inner.is_poisoned(), "and the lock is healthy again, so the warning is not repeated");
        assert_eq!(handler.inner.lock().delivery_source_uid, "", "a half-finished delivery's sender is not left behind");
    }

    #[test]
    fn a_system_note_that_panics_in_the_sender_costs_only_that_note() {
        let handler = ReactiveHandler::new();
        handler.set_message_sender(sender_that_panics_once());
        handler.inner.lock().register_agent("agent1", "block1", None).unwrap();
        assert!(panics_on_a_thread(|| {
            let _ = handler.deliver_system_note("agent1", "[AgentMux] idle");
        }));
        handler.deliver_system_note("agent1", "[AgentMux] again").expect("the next note is delivered");
        assert_eq!(handler.list_agents().len(), 1);
        let notes = handler.get_audit_log(10).iter().filter(|entry| entry.outcome.as_deref() == Some("system_note")).count();
        assert_eq!(notes, 1, "only the note that was delivered is audited; the one that panicked never reached its audit row");
    }

    #[test]
    fn a_jekt_that_panics_mid_delivery_leaves_no_sender_on_the_next_audit_entry() {
        let handler = ReactiveHandler::new();
        handler.set_message_sender(sender_that_panics_once());
        handler.inner.lock().register_agent("agentg", "block1", None).unwrap();
        let request = |uid: &str| InjectionRequest {
            target_agent: "agentg".to_string(),
            message: "hello".to_string(),
            audit_source_uid: uid.to_string(),
            lan_claimed_uid: uid.to_string(),
            ..Default::default()
        };
        assert!(panics_on_a_thread(|| {
            handler.inject_message(request("uid-of-the-first-sender"));
        }));
        let resp = handler.inject_message(request(""));
        assert!(resp.success, "{:?}", resp.error);
        assert_eq!(handler.inner.lock().delivery_source_uid, "");
        assert_eq!(handler.inner.lock().delivery_lan_claimed_uid, "");
        assert!(
            handler.get_audit_log(5).iter().all(|entry| !format!("{entry:?}").contains("uid-of-the-first-sender")),
            "no audit entry carries the failed delivery's sender"
        );
    }

    #[test]
    fn the_spawn_path_locks_recover_too() {
        let handler = ReactiveHandler::new();
        handler.inner.lock().register_agent("agent1", "block1", None).unwrap();
        assert!(panics_on_a_thread(|| {
            let _guard = handler.inner.lock();
            panic!("a panic while the lock is held");
        }));
        // These two answered "mutex poisoned" and None for the rest of the process's life.
        assert!(handler.try_get_agent_by_block("block1").is_some());
        assert!(!handler.inner.is_poisoned());
        assert!(panics_on_a_thread(|| {
            let _guard = handler.inner.lock();
            panic!("and again");
        }));
        handler.try_register_agent_with_nonce("agent2", "block2", None, 1, None).expect("the spawn-path registration goes through");
    }

    #[test]
    fn every_recovery_is_counted_for_health() {
        let handler = ReactiveHandler::new();
        assert_eq!(handler.recovered_lock_count(), 0);
        for expected in 1..=2u64 {
            assert!(panics_on_a_thread(|| {
                let _guard = handler.inner.lock();
                panic!("a panic while the lock is held");
            }));
            assert_eq!(handler.list_agents().len(), 0);
            assert_eq!(handler.recovered_lock_count(), expected, "one recovery per poisoning, not one per later call");
            handler.list_agents();
            assert_eq!(handler.recovered_lock_count(), expected, "a healthy lock adds nothing");
        }
    }

    #[test]
    fn the_lock_is_free_unless_someone_holds_it() {
        let handler = ReactiveHandler::new();
        assert!(handler.lock_is_free());
        let guard = handler.inner.lock();
        assert!(!handler.lock_is_free(), "a lock held for good (a stuck delivery, a deadlock) is not free: health says so");
        drop(guard);
        assert!(handler.lock_is_free());
        // A poisoned lock is recovered by the probe itself, and counted.
        assert!(panics_on_a_thread(|| {
            let _guard = handler.inner.lock();
            panic!("a panic while the lock is held");
        }));
        assert!(handler.lock_is_free());
        assert_eq!(handler.recovered_lock_count(), 1);
    }
}
