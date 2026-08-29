// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Parked credential-approval registry.
//!
//! Mirrors [`crate::browser_pane::auth`]'s TTL/epoch/HashMap shape, but is
//! a distinct registry, not a shared one — the two have different
//! cleanup triggers (this one's entries die with the *approval subwindow*;
//! `browser_pane::auth`'s entries die with the *pane*), and a
//! forced-generic registry covering both would be worse than two small
//! ones. See `docs/status/majestic-painting-minsky` plan, "Why this
//! shape."
//!
//! One entry here represents one *decision* pending human input — not one
//! CEF auth challenge. Multiple simultaneous challenges for the same
//! `(identity_id, origin, realm, is_proxy)` protection space coalesce
//! into a single entry (and a single approval subwindow): see
//! [`reserve_or_join`].
//!
//! The actual parked CEF `AuthCallback`s stay in `browser_pane::auth`,
//! keyed by the `request_id`s this module only stores by string — this
//! module never touches a CEF type directly, so it has no IO-thread
//! affinity concerns of its own.

use cef::ImplAuthCallback;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// One `browser_pane::auth` request riding this approval — its
/// `request_id` (to resolve/cancel the parked `AuthCallback` later) and
/// the `block_id` of the pane that raised it (so a pane closing
/// independently of the approval window can be un-joined without
/// disturbing siblings still waiting on the same approval).
struct AuthRequest {
    request_id: String,
    block_id: String,
}

struct Entry {
    identity_id: String,
    origin: String,
    realm: String,
    is_proxy: bool,
    /// `None` until the approval subwindow has actually been created —
    /// see [`reserve_or_join`]'s doc comment for why reservation and
    /// window-creation are two separate steps.
    window_id: Option<String>,
    auth_requests: Vec<AuthRequest>,
    epoch: u64,
}

fn pending() -> &'static Mutex<HashMap<String, Entry>> {
    static CELL: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(0);

/// A single click, not manual multi-field entry — shorter than
/// `browser_pane::auth`'s 5-minute TTL, but still generous for a human to
/// actually notice and act on an approval prompt.
const PARKED_TTL: Duration = Duration::from_secs(60);

pub enum Reservation {
    /// Joined an already-pending approval for the same protection space —
    /// caller must NOT open a second approval subwindow. When the existing
    /// approval resolves, this request resolves with it.
    Joined,
    /// No existing approval for this protection space; caller now owns
    /// creating the subwindow and must call [`finalize_window`] with the
    /// resulting `window_id` once it exists (or [`abandon`] if window
    /// creation fails, so this one request can fall through to the normal
    /// prompt instead of hanging until TTL).
    New { approval_id: String },
}

/// Reserve (or join) a pending approval for `(identity_id, origin, realm,
/// is_proxy)`. Reservation happens BEFORE the subwindow actually exists —
/// opening a CEF window is itself async, and if a second challenge for the
/// same protection space arrived in that gap, it would otherwise miss the
/// yet-to-be-registered entry and open a redundant second window. Callers
/// must always resolve a `New` reservation via `finalize_window` or
/// `abandon`.
pub fn reserve_or_join(
    identity_id: &str,
    origin: &str,
    realm: &str,
    is_proxy: bool,
    request_id: String,
    block_id: String,
) -> Reservation {
    let mut g = pending().lock();
    if let Some(entry) = g.values_mut().find(|e| {
        e.identity_id == identity_id && e.origin == origin && e.realm == realm && e.is_proxy == is_proxy
    }) {
        entry.auth_requests.push(AuthRequest { request_id, block_id });
        return Reservation::Joined;
    }

    let approval_id = uuid::Uuid::new_v4().to_string();
    let epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
    g.insert(
        approval_id.clone(),
        Entry {
            identity_id: identity_id.to_string(),
            origin: origin.to_string(),
            realm: realm.to_string(),
            is_proxy,
            window_id: None,
            auth_requests: vec![AuthRequest { request_id, block_id }],
            epoch,
        },
    );
    arm_ttl(approval_id.clone(), epoch);
    Reservation::New { approval_id }
}

/// Record the approval subwindow's `window_id` once it has actually been
/// created, so `cancel_for_window` can later find this entry when the
/// window (or its parent, cascading down) closes.
/// Returns `false` when the approval was already gone — the reservation was
/// cancelled (pane closed) or timed out during the window-creation gap, so
/// the window the caller just opened belongs to nothing and **the caller
/// must close it**. `#[must_use]` because ignoring that answer is exactly
/// the bug this return value exists to prevent (reagent P1 on PR #2824):
/// the entry is gone, so `cancel_for_window` will never match it, the IPC
/// decide-handler's `take` returns `None` and closes nothing, and the TTL
/// timer has no entry left to fire on — leaving a dead, unresponsive
/// subwindow open until the human closes it by hand.
#[must_use]
pub fn finalize_window(approval_id: &str, window_id: String) -> bool {
    let mut g = pending().lock();
    if let Some(e) = g.get_mut(approval_id) {
        e.window_id = Some(window_id);
        true
    } else {
        tracing::warn!(
            "[credential-broker] finalize_window: approval {} no longer pending \
             (already resolved or timed out) — the just-opened subwindow is \
             orphaned; closing it",
            approval_id
        );
        false
    }
}

/// A resolved approval — every parked auth request that was riding it,
/// the window that should now be closed (its job is done), and the
/// protection-space fields the caller needs to make the `credential.Fill`
/// call on approve (this registry never holds the password itself — only
/// enough to ask srv for it once a human has said yes).
pub struct ResolvedApproval {
    pub identity_id: String,
    pub origin: String,
    pub realm: String,
    pub is_proxy: bool,
    pub auth_request_ids: Vec<String>,
    pub window_id: Option<String>,
}

/// Take (remove) a pending approval, e.g. after the human clicks
/// Approve/Deny in the subwindow. `None` if it was already resolved or
/// timed out — the IPC handler treats that as "nothing to do, just close
/// the window."
pub fn take(approval_id: &str) -> Option<ResolvedApproval> {
    pending().lock().remove(approval_id).map(|e| ResolvedApproval {
        identity_id: e.identity_id,
        origin: e.origin,
        realm: e.realm,
        is_proxy: e.is_proxy,
        auth_request_ids: e.auth_requests.into_iter().map(|r| r.request_id).collect(),
        window_id: e.window_id,
    })
}

/// Abandon a `New` reservation whose window creation failed — removes the
/// entry outright (there's no window to ever finalize) and returns the
/// request ids so the caller can fall each through to the normal prompt.
pub fn abandon(approval_id: &str) -> Vec<String> {
    pending()
        .lock()
        .remove(approval_id)
        .map(|e| e.auth_requests.into_iter().map(|r| r.request_id).collect())
        .unwrap_or_default()
}

/// Cancel whichever pending approval owns `window_id` — called when the
/// approval subwindow itself closes (directly, or cascaded from its
/// parent closing) before the human decided. Returns the auth request ids
/// so the caller can cancel each parked `browser_pane::auth` callback
/// (page falls back to its normal 401 body, same as clicking Cancel on
/// the manual prompt today).
pub fn cancel_for_window(window_id: &str) -> Vec<String> {
    let mut g = pending().lock();
    let approval_id = g
        .iter()
        .find(|(_, e)| e.window_id.as_deref() == Some(window_id))
        .map(|(k, _)| k.clone());
    match approval_id {
        Some(id) => g
            .remove(&id)
            .map(|e| e.auth_requests.into_iter().map(|r| r.request_id).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Un-join `block_id`'s auth request(s) from whichever pending approval(s)
/// they're riding — called when a pane closes independently of the
/// approval window (e.g. one of two coalesced panes closes while the
/// human hasn't decided yet). If removing them empties an entry
/// entirely, that whole approval is cancelled too (nothing left to
/// resolve) and its `window_id` (if the subwindow already exists) is
/// returned so the caller can close it.
/// Returns **every** emptied approval's `window_id`, not just the first.
///
/// The earlier cut `break`'d on the first entry that emptied, which silently
/// skipped the rest (reagent P2 on PR #2824). One pane can legitimately ride
/// two simultaneous approvals for *different* protection spaces — a
/// proxy-auth and an origin-auth challenge from the same page is the obvious
/// case — and `HashMap` iteration order decides which one the `break` lands
/// on. The others kept a stale `AuthRequest` for an already-closed pane, and
/// their subwindows stayed open, until the 60s TTL swept them.
pub fn cancel_for_block(block_id: &str) -> Vec<String> {
    let mut g = pending().lock();
    let mut emptied_ids: Vec<String> = Vec::new();
    for (id, entry) in g.iter_mut() {
        let before = entry.auth_requests.len();
        entry.auth_requests.retain(|r| r.block_id != block_id);
        if entry.auth_requests.len() != before && entry.auth_requests.is_empty() {
            emptied_ids.push(id.clone());
        }
    }
    emptied_ids
        .into_iter()
        .filter_map(|id| g.remove(&id))
        .filter_map(|e| e.window_id)
        .collect()
}

/// Same rationale as `browser_pane::auth`'s own TTL timer — this can be
/// armed from CEF's IO thread (via `on_auth_credentials`'s spawned
/// orchestration task, which itself needed a handle to spawn from in the
/// first place), so a bare `tokio::spawn` is not guaranteed safe here
/// either. Shares the single runtime handle installed by
/// `super::set_runtime_handle` rather than keeping a second copy.
fn arm_ttl(approval_id: String, epoch: u64) {
    match super::runtime_handle() {
        Some(handle) => {
            handle.spawn(async move {
                tokio::time::sleep(PARKED_TTL).await;
                let expired: Option<Entry> = {
                    let mut g = pending().lock();
                    match g.get(&approval_id) {
                        Some(e) if e.epoch == epoch => g.remove(&approval_id),
                        _ => None,
                    }
                };
                if let Some(entry) = expired {
                    tracing::warn!(
                        "[credential-broker] approval {} timed out after {:?} — \
                         cancelling {} parked auth request(s)",
                        approval_id,
                        PARKED_TTL,
                        entry.auth_requests.len(),
                    );
                    for req in entry.auth_requests {
                        if let Some(cb) = crate::browser_pane::auth::take(&req.request_id) {
                            cb.cancel();
                        }
                    }
                    // Close the subwindow too. It used to be left for "the
                    // IPC decide-handler to close on the next interaction"
                    // — but that handler's `take` returns `None` for an
                    // already-expired approval and returns without closing
                    // anything, so a late Approve/Deny click did nothing
                    // and the window sat inert until closed by hand
                    // (reagent P2 on PR #2824). Routed through
                    // `super::close_approval_window` rather than calling
                    // `commands::window` here, so this registry keeps its
                    // design property of holding only `window_id` strings.
                    if let Some(window_id) = entry.window_id {
                        super::close_approval_window(&window_id);
                    }
                }
            });
        }
        None => {
            tracing::error!(
                "[credential-broker] tokio handle not installed; TTL timer skipped for \
                 approval {} — entry will leak if never resolved. Did main.rs call \
                 credential_broker::approval::set_runtime_handle?",
                approval_id
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // All tests mutate the same process-wide static registry — cargo runs
    // tests in a binary concurrently by default, so without serializing
    // them here two tests could interleave against the same HashMap and
    // produce flaky failures unrelated to the logic under test.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn drain() {
        pending().lock().clear();
    }

    #[test]
    fn reserve_creates_new_entry_for_first_request() {
        let _guard = TEST_LOCK.lock();
        drain();
        match reserve_or_join("id-a", "https://a", "realm", false, "req-1".into(), "block-1".into()) {
            Reservation::New { approval_id } => assert!(!approval_id.is_empty()),
            Reservation::Joined => panic!("expected New for first request"),
        }
        drain();
    }

    #[test]
    fn second_request_for_same_protection_space_joins() {
        let _guard = TEST_LOCK.lock();
        drain();
        let approval_id = match reserve_or_join("id-a", "https://a", "realm", false, "req-1".into(), "block-1".into())
        {
            Reservation::New { approval_id } => approval_id,
            Reservation::Joined => panic!("expected New for first request"),
        };
        match reserve_or_join("id-a", "https://a", "realm", false, "req-2".into(), "block-2".into()) {
            Reservation::Joined => {}
            Reservation::New { .. } => panic!("expected Joined for coalescing second request"),
        }
        let resolved = take(&approval_id).expect("entry should exist");
        assert_eq!(resolved.auth_request_ids.len(), 2);
        drain();
    }

    #[test]
    fn different_protection_spaces_do_not_coalesce() {
        let _guard = TEST_LOCK.lock();
        drain();
        reserve_or_join("id-a", "https://a", "realm", false, "req-1".into(), "block-1".into());
        match reserve_or_join("id-a", "https://b", "realm", false, "req-2".into(), "block-2".into()) {
            Reservation::New { .. } => {}
            Reservation::Joined => panic!("different origins must not coalesce"),
        }
        drain();
    }

    #[test]
    fn different_identities_do_not_coalesce_even_for_same_origin() {
        let _guard = TEST_LOCK.lock();
        drain();
        reserve_or_join("id-a", "https://a", "realm", false, "req-1".into(), "block-1".into());
        match reserve_or_join("id-b", "https://a", "realm", false, "req-2".into(), "block-2".into()) {
            Reservation::New { .. } => {}
            Reservation::Joined => panic!("different identities must not coalesce"),
        }
        drain();
    }

    #[test]
    fn take_removes_entry_and_returns_window_id() {
        let _guard = TEST_LOCK.lock();
        drain();
        let approval_id = match reserve_or_join("id-a", "https://a", "realm", false, "req-1".into(), "block-1".into())
        {
            Reservation::New { approval_id } => approval_id,
            Reservation::Joined => unreachable!(),
        };
        assert!(finalize_window(&approval_id, "win-1".into()));
        let resolved = take(&approval_id).expect("entry should exist");
        assert_eq!(resolved.window_id.as_deref(), Some("win-1"));
        assert!(take(&approval_id).is_none(), "take must remove the entry");
        drain();
    }

    #[test]
    fn cancel_for_window_finds_entry_by_window_id_not_approval_id() {
        let _guard = TEST_LOCK.lock();
        drain();
        let approval_id = match reserve_or_join("id-a", "https://a", "realm", false, "req-1".into(), "block-1".into())
        {
            Reservation::New { approval_id } => approval_id,
            Reservation::Joined => unreachable!(),
        };
        assert!(finalize_window(&approval_id, "win-1".into()));
        let ids = cancel_for_window("win-1");
        assert_eq!(ids, vec!["req-1".to_string()]);
        assert!(take(&approval_id).is_none());
        drain();
    }

    #[test]
    fn cancel_for_block_unjoins_without_disturbing_sibling() {
        let _guard = TEST_LOCK.lock();
        drain();
        let approval_id = match reserve_or_join("id-a", "https://a", "realm", false, "req-1".into(), "block-1".into())
        {
            Reservation::New { approval_id } => approval_id,
            Reservation::Joined => unreachable!(),
        };
        reserve_or_join("id-a", "https://a", "realm", false, "req-2".into(), "block-2".into());
        let closed_window = cancel_for_block("block-1");
        assert!(closed_window.is_empty(), "sibling still pending — window must not close");
        let resolved = take(&approval_id).expect("entry should still exist for block-2's request");
        assert_eq!(resolved.auth_request_ids, vec!["req-2".to_string()]);
        drain();
    }

    #[test]
    fn cancel_for_block_closes_window_when_last_request_leaves() {
        let _guard = TEST_LOCK.lock();
        drain();
        let approval_id = match reserve_or_join("id-a", "https://a", "realm", false, "req-1".into(), "block-1".into())
        {
            Reservation::New { approval_id } => approval_id,
            Reservation::Joined => unreachable!(),
        };
        assert!(finalize_window(&approval_id, "win-1".into()));
        let closed_window = cancel_for_block("block-1");
        assert_eq!(closed_window, vec!["win-1".to_string()]);
        assert!(take(&approval_id).is_none());
        drain();
    }

    #[test]
    fn abandon_removes_entry_and_returns_request_ids() {
        let _guard = TEST_LOCK.lock();
        drain();
        let approval_id = match reserve_or_join("id-a", "https://a", "realm", false, "req-1".into(), "block-1".into())
        {
            Reservation::New { approval_id } => approval_id,
            Reservation::Joined => unreachable!(),
        };
        let ids = abandon(&approval_id);
        assert_eq!(ids, vec!["req-1".to_string()]);
        assert!(take(&approval_id).is_none());
        drain();
    }

    /// reagent P2 on PR #2824: the old `cancel_for_block` `break`'d on the
    /// first entry that emptied, so a pane riding TWO approvals for different
    /// protection spaces (proxy-auth + origin-auth from the same page) left
    /// the second one's stale request registered — and its window open —
    /// until the 60s TTL swept it. Which one survived depended on HashMap
    /// iteration order, so this failed intermittently rather than never.
    #[test]
    fn cancel_for_block_empties_every_approval_the_block_was_riding() {
        let _guard = TEST_LOCK.lock();
        drain();

        let origin_approval = match reserve_or_join("id-a", "https://a", "realm", false, "req-1".into(), "block-1".into()) {
            Reservation::New { approval_id } => approval_id,
            Reservation::Joined => unreachable!("first request for this space"),
        };
        assert!(finalize_window(&origin_approval, "win-origin".into()));

        // Same block, DIFFERENT protection space (is_proxy) — a separate entry.
        let proxy_approval = match reserve_or_join("id-a", "https://a", "realm", true, "req-2".into(), "block-1".into()) {
            Reservation::New { approval_id } => approval_id,
            Reservation::Joined => unreachable!("different protection space must not coalesce"),
        };
        assert!(finalize_window(&proxy_approval, "win-proxy".into()));

        let mut closed = cancel_for_block("block-1");
        closed.sort();
        assert_eq!(
            closed,
            vec!["win-origin".to_string(), "win-proxy".to_string()],
            "both approvals emptied, so both windows must be returned for closing",
        );
        assert!(take(&origin_approval).is_none(), "origin approval must be gone");
        assert!(take(&proxy_approval).is_none(), "proxy approval must be gone");
        drain();
    }

    /// reagent P1 on PR #2824: no lock is held across `open_approval_window`,
    /// so the reservation can be cancelled in that gap. `finalize_window`
    /// must report that, or the caller leaves a window nothing will ever
    /// close — `cancel_for_window` can't match it, the decide-handler's
    /// `take` returns `None`, and the TTL entry is already gone.
    #[test]
    fn finalize_window_reports_a_reservation_that_vanished_during_the_gap() {
        let _guard = TEST_LOCK.lock();
        drain();

        let approval_id = match reserve_or_join("id-a", "https://a", "realm", false, "req-1".into(), "block-1".into()) {
            Reservation::New { approval_id } => approval_id,
            Reservation::Joined => unreachable!(),
        };
        // The pane closes while the subwindow is still being created.
        let closed = cancel_for_block("block-1");
        assert!(closed.is_empty(), "no window_id yet — nothing for the caller to close from here");

        assert!(
            !finalize_window(&approval_id, "win-orphan".into()),
            "the approval is gone, so the caller owns closing the window it just opened",
        );
        drain();
    }

    /// The ordinary path still reports success, so the caller doesn't close a
    /// window that is legitimately in use.
    #[test]
    fn finalize_window_reports_success_for_a_live_reservation() {
        let _guard = TEST_LOCK.lock();
        drain();
        let approval_id = match reserve_or_join("id-a", "https://a", "realm", false, "req-1".into(), "block-1".into()) {
            Reservation::New { approval_id } => approval_id,
            Reservation::Joined => unreachable!(),
        };
        assert!(finalize_window(&approval_id, "win-1".into()));
        assert_eq!(take(&approval_id).and_then(|r| r.window_id).as_deref(), Some("win-1"));
        drain();
    }
}
