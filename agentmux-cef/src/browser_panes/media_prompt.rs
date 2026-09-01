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

/// How long a prompt may stay unanswered before it is denied.
///
/// Long enough for a person to read and decide; short enough that a page whose
/// prompt never rendered fails in a comprehensible amount of time rather than
/// appearing to hang. A denial here is not recorded as a grant, so a later
/// request prompts again.
const PROMPT_TIMEOUT_MS: i64 = 60_000;

thread_local! {
    /// UI-thread-only registry of parked requests, keyed by request id.
    static PENDING: RefCell<HashMap<u64, PendingRequest>> = RefCell::new(HashMap::new());

    /// Monotonic source of request ids. Thread-local like the map it keys, so
    /// there is no cross-thread counter to reason about.
    static NEXT_ID: RefCell<u64> = const { RefCell::new(1) };
}

struct PendingRequest {
    callback: MediaAccessCallback,
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
    let id = NEXT_ID.with(|n| {
        let mut n = n.borrow_mut();
        let id = *n;
        *n += 1;
        id
    });
    PENDING.with(|p| {
        p.borrow_mut().insert(
            id,
            PendingRequest {
                callback,
                block_id: block_id.to_string(),
                origin: origin.to_string(),
                requested,
            },
        );
    });
    id
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
    // Remove BEFORE continuing: the entry must be unreachable by the time the
    // callback is used, so no second path can continue it again.
    let Some(req) = PENDING.with(|p| p.borrow_mut().remove(&id)) else {
        return None;
    };
    let allowed = if allow { req.requested } else { 0 };
    req.callback.cont(allowed);
    tracing::info!(
        target: "pane-media",
        request_id = id,
        block_id = %req.block_id,
        origin = %req.origin,
        requested = req.requested,
        allowed,
        "resolved media permission prompt"
    );
    Some((req.block_id, req.origin, req.requested))
}

/// Deny every pending request belonging to one pane.
///
/// Called on pane close: the pane the answer would apply to is gone, so there
/// is nothing a later "allow" could sensibly mean.
pub fn cancel_pane(block_id: &str) {
    debug_assert_ne!(cef::currently_on(cef::ThreadId::UI), 0);
    let ids: Vec<u64> = PENDING.with(|p| {
        p.borrow()
            .iter()
            .filter(|(_, r)| r.block_id == block_id)
            .map(|(id, _)| *id)
            .collect()
    });
    for id in ids {
        resolve(id, false);
    }
}

/// Cancel a pane's prompts from any thread.
///
/// The registry is UI-thread-only, but teardown paths are not guaranteed to run
/// there — so this hops to the UI thread rather than making every caller prove
/// which thread it is on. Posting even when already on the UI thread is
/// deliberate: it keeps one code path, and the timeout is the backstop if the
/// hop is somehow never serviced.
pub fn cancel_pane_any_thread(block_id: &str) {
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
    PENDING.with(|p| p.borrow().contains_key(&id))
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
