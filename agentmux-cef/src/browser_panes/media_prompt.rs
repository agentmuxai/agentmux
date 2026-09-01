// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-flight media permission prompts for browser panes.
//!
//! Spec: `docs/specs/SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md` §3.4-3.5.
//!
//! # Why a thread-local rather than `AppState`
//!
//! `CefMediaAccessCallback` is a `RefGuard` with **no `Send`/`Sync`**, so it
//! cannot be stored in `AppState` (shared across threads). It is also a CEF
//! object that must be used on the browser-process UI thread.
//!
//! Both constraints point the same way: keep pending requests in a thread-local
//! owned by the UI thread, and route every mutation through a task posted to
//! `ThreadId::UI`. `debug_assert`s below pin that invariant so a future caller
//! on the wrong thread fails loudly in dev rather than corrupting the map.
//!
//! # The invariant that matters
//!
//! **Every parked request is resolved exactly once, and always eventually.**
//!
//! - Resolution *removes* the entry before continuing the callback, so a
//!   double-answer (user clicks, then a timeout fires) cannot continue the same
//!   callback twice — that would be a use-after-continue into CEF.
//! - Every park arms a timeout, so a frontend that never answers (window
//!   closed, renderer crashed, event dropped) degrades to a denial rather than
//!   hanging the page's `getUserMedia` forever.
//! - Pane close cancels that pane's pending requests, since the answer can no
//!   longer be attributed to anything.

use std::cell::RefCell;
use std::collections::HashMap;

use cef::*;

/// The one thing a parked request can have done to it.
///
/// Exists so the registry's exactly-once logic can be tested without a real
/// `MediaAccessCallback` — that type needs a live CEF process, which
/// `cargo test` does not have (the same constraint recorded in PR #2890). The
/// invariant this module rests on is worth more than the indirection costs.
pub trait MediaContinuation {
    fn continue_with(&self, allowed: u32);
}

impl MediaContinuation for MediaAccessCallback {
    fn continue_with(&self, allowed: u32) {
        self.cont(allowed);
    }
}

/// How long a prompt may stay unanswered before it is denied.
///
/// Long enough for a person to read and decide; short enough that a page whose
/// prompt never rendered fails in a comprehensible amount of time rather than
/// appearing to hang. A denial here is not recorded as a grant, so a later
/// request prompts again.
const PROMPT_TIMEOUT_MS: i64 = 60_000;

thread_local! {
    /// UI-thread-only registry of parked requests.
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// The park/resolve/cancel bookkeeping, with no threading or CEF in it.
///
/// Split out purely so it can be tested — the public functions below add the
/// UI-thread assertions and the thread-local, neither of which a unit test can
/// satisfy.
#[derive(Default)]
struct Registry {
    pending: HashMap<u64, PendingRequest>,
    next_id: u64,
}

impl Registry {
    fn park(
        &mut self,
        callback: Box<dyn MediaContinuation>,
        block_id: &str,
        origin: &str,
        requested: u32,
    ) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.pending.insert(
            id,
            PendingRequest {
                callback,
                block_id: block_id.to_string(),
                origin: origin.to_string(),
                requested,
            },
        );
        id
    }

    /// Remove BEFORE continuing, so no second path can continue the same
    /// callback twice — that would be a use-after-continue into CEF.
    fn resolve(&mut self, id: u64, allow: bool) -> Option<(String, String, u32)> {
        let req = self.pending.remove(&id)?;
        let allowed = if allow { req.requested } else { 0 };
        req.callback.continue_with(allowed);
        Some((req.block_id, req.origin, req.requested))
    }

    fn ids_for_pane(&self, block_id: &str) -> Vec<u64> {
        self.pending
            .iter()
            .filter(|(_, r)| r.block_id == block_id)
            .map(|(id, _)| *id)
            .collect()
    }

    fn is_pending(&self, id: u64) -> bool {
        self.pending.contains_key(&id)
    }
}

struct PendingRequest {
    callback: Box<dyn MediaContinuation>,
    block_id: String,
    origin: String,
    /// Exactly the bits the page asked for. Echoed verbatim on allow — CEF
    /// requires `allowed_permissions` to match `required_permissions`, so a
    /// grant is all-or-nothing for the request as posed.
    requested: u32,
}

/// Park a request and return its id, for handing to the prompt UI.
///
/// The caller must have already decided that a prompt is warranted (no existing
/// grant covers the request) and must emit the prompt event *after* this
/// returns, so an implausibly fast answer still finds the entry.
pub fn park(callback: MediaAccessCallback, block_id: &str, origin: &str, requested: u32) -> u64 {
    debug_assert_ne!(
        cef::currently_on(cef::ThreadId::UI),
        0,
        "media prompts must be parked on the CEF UI thread"
    );
    REGISTRY.with(|r| r.borrow_mut().park(Box::new(callback), block_id, origin, requested))
}

/// Resolve a parked request. `None` if it is already gone — an unknown id, a
/// double answer, or a timeout that already fired. Silent by design: all three
/// are ordinary races, not errors.
///
/// Returns `(block_id, origin, requested)` on success so the caller can record
/// a grant, which this module deliberately does not do itself: parking is
/// mechanism, granting is policy, and the grant store owns policy.
pub fn resolve(id: u64, allow: bool) -> Option<(String, String, u32)> {
    debug_assert_ne!(
        cef::currently_on(cef::ThreadId::UI),
        0,
        "media prompts must be resolved on the CEF UI thread"
    );
    let out = REGISTRY.with(|r| r.borrow_mut().resolve(id, allow));
    if let Some((block_id, origin, requested)) = out.as_ref() {
        tracing::info!(
            target: "pane-media",
            request_id = id,
            %block_id, %origin, requested,
            allowed = if allow { *requested } else { 0 },
            "resolved media permission prompt"
        );
    }
    out
}

/// Deny every pending request belonging to one pane.
///
/// Called on pane close: the pane the answer would apply to is gone, so there
/// is nothing a later "allow" could sensibly mean.
pub fn cancel_pane(block_id: &str) {
    debug_assert_ne!(cef::currently_on(cef::ThreadId::UI), 0);
    let ids: Vec<u64> = REGISTRY.with(|r| r.borrow().ids_for_pane(block_id));
    for id in ids {
        resolve(id, false);
    }
}

/// Cancel a pane's prompts from any thread.
///
/// **Cancels synchronously when already on the UI thread**, which is the case
/// that matters. Teardown clears a pane's grants synchronously and may then
/// recreate a pane for the same block id in the very next statement
/// (`replay_pending_create`). If cancellation were merely *posted*, a pending
/// "allow" could still be resolved after that replay and grant the NEW pane
/// access the user never approved for it — the exact hazard the clear-before-
/// replay ordering exists to prevent (reagent P2 on PR #2899).
///
/// Off the UI thread there is no way to touch the registry directly, so it
/// posts. That path is strictly best-effort ordering-wise; the timeout remains
/// the backstop.
pub fn cancel_pane_any_thread(block_id: &str) {
    if cef::currently_on(cef::ThreadId::UI) != 0 {
        cancel_pane(block_id);
        return;
    }
    let mut task = CancelPaneTask::new(block_id.to_string());
    cef::post_task(cef::ThreadId::UI, Some(&mut task));
}

wrap_task! {
    pub(crate) struct CancelPaneTask {
        block_id: String,
    }

    impl Task {
        fn execute(&self) {
            cancel_pane(&self.block_id);
        }
    }
}

/// Apply a user's answer on the UI thread, recording a grant when allowed.
///
/// The grant is recorded only on the resolve path that actually found a live
/// entry — a duplicate or timed-out answer must not retroactively create a
/// grant for a request that was already denied.
wrap_task! {
    pub(crate) struct RespondTask {
        id: u64,
        allow: bool,
        state: std::sync::Arc<crate::state::AppState>,
    }

    impl Task {
        fn execute(&self) {
            let Some((block_id, origin, requested)) = resolve(self.id, self.allow) else {
                return; // unknown id, double answer, or already timed out
            };
            if self.allow {
                self.state
                    .media_grants
                    .lock()
                    .grant(&block_id, &origin, requested);
                tracing::info!(
                    target: "pane-media",
                    %block_id, %origin, requested,
                    "recorded media grant"
                );
            }
        }
    }
}

/// True while `id` is still awaiting an answer. For the timeout task.
fn is_pending(id: u64) -> bool {
    REGISTRY.with(|r| r.borrow().is_pending(id))
}

/// Arm the safety-net denial for a parked request.
///
/// Split from [`park`] so the caller emits its prompt event first; the delay
/// dwarfs that ordering either way, but the dependency is explicit.
pub fn arm_timeout(id: u64) {
    let mut task = TimeoutTask::new(id);
    cef::post_delayed_task(cef::ThreadId::UI, Some(&mut task), PROMPT_TIMEOUT_MS);
}

wrap_task! {
    pub(crate) struct TimeoutTask {
        id: u64,
    }

    impl Task {
        fn execute(&self) {
            if !is_pending(self.id) {
                return; // already answered — the common case
            }
            tracing::warn!(
                target: "pane-media",
                request_id = self.id,
                timeout_ms = PROMPT_TIMEOUT_MS,
                "media permission prompt went unanswered — denying"
            );
            resolve(self.id, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    /// Records every continuation, so a double-continue is observable rather
    /// than silently fine.
    #[derive(Default)]
    struct FakeCallback {
        calls: Rc<RefCell<Vec<u32>>>,
    }

    impl MediaContinuation for FakeCallback {
        fn continue_with(&self, allowed: u32) {
            self.calls.borrow_mut().push(allowed);
        }
    }

    fn park(reg: &mut Registry, block: &str, requested: u32) -> (u64, Rc<RefCell<Vec<u32>>>) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let cb = FakeCallback { calls: calls.clone() };
        let id = reg.park(Box::new(cb), block, "https://example.com", requested);
        (id, calls)
    }

    const VIDEO: u32 = 1 << 1;

    #[test]
    fn allow_continues_with_exactly_the_requested_bits() {
        // CEF requires allowed_permissions to MATCH required_permissions —
        // echoing anything else denies the whole request.
        let mut reg = Registry::default();
        let (id, calls) = park(&mut reg, "b1", VIDEO);
        reg.resolve(id, true);
        assert_eq!(*calls.borrow(), vec![VIDEO]);
    }

    #[test]
    fn deny_continues_with_zero() {
        let mut reg = Registry::default();
        let (id, calls) = park(&mut reg, "b1", VIDEO);
        reg.resolve(id, false);
        assert_eq!(*calls.borrow(), vec![0]);
    }

    #[test]
    fn a_request_is_continued_exactly_once_even_if_answered_twice() {
        // THE invariant. A user answer racing the timeout must not continue the
        // same CEF callback twice — that would be a use-after-continue.
        let mut reg = Registry::default();
        let (id, calls) = park(&mut reg, "b1", VIDEO);

        assert!(reg.resolve(id, true).is_some());
        assert!(reg.resolve(id, true).is_none(), "second answer must be a no-op");
        assert!(reg.resolve(id, false).is_none(), "and so must a later deny");

        assert_eq!(*calls.borrow(), vec![VIDEO], "continued exactly once");
    }

    #[test]
    fn resolving_an_unknown_id_is_a_silent_no_op() {
        let mut reg = Registry::default();
        assert!(reg.resolve(999, true).is_none());
    }

    #[test]
    fn resolve_reports_what_is_needed_to_record_a_grant() {
        let mut reg = Registry::default();
        let (id, _) = park(&mut reg, "b1", VIDEO);
        let out = reg.resolve(id, true).expect("resolved");
        assert_eq!(out, ("b1".to_string(), "https://example.com".to_string(), VIDEO));
    }

    #[test]
    fn ids_are_unique_across_parks() {
        let mut reg = Registry::default();
        let (a, _) = park(&mut reg, "b1", VIDEO);
        let (b, _) = park(&mut reg, "b1", VIDEO);
        assert_ne!(a, b, "a reused id would let one answer resolve the wrong request");
    }

    #[test]
    fn cancelling_a_pane_denies_only_that_panes_requests() {
        let mut reg = Registry::default();
        let (keep, keep_calls) = park(&mut reg, "other", VIDEO);
        let (a, a_calls) = park(&mut reg, "doomed", VIDEO);
        let (b, b_calls) = park(&mut reg, "doomed", VIDEO);

        for id in reg.ids_for_pane("doomed") {
            reg.resolve(id, false);
        }

        assert_eq!(*a_calls.borrow(), vec![0]);
        assert_eq!(*b_calls.borrow(), vec![0]);
        assert!(keep_calls.borrow().is_empty(), "other panes untouched");
        assert!(!reg.is_pending(a) && !reg.is_pending(b));
        assert!(reg.is_pending(keep));
    }

    #[test]
    fn is_pending_tracks_the_lifecycle_the_timeout_depends_on() {
        // The timeout task skips already-answered requests using this; if it
        // reported stale state the timeout would continue a dead callback.
        let mut reg = Registry::default();
        let (id, _) = park(&mut reg, "b1", VIDEO);
        assert!(reg.is_pending(id));
        reg.resolve(id, true);
        assert!(!reg.is_pending(id));
    }
}
