// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-pane, per-origin media capture grants.
//!
//! Spec: `docs/specs/SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md` §3.1-3.2.
//!
//! # Why the grant records a bitmask rather than a set of booleans
//!
//! CEF requires `allowed_permissions` to **match** `required_permissions` for a
//! `getUserMedia` request — granting a subset is a denial of the whole request,
//! not a partial grant (`cef_permission_handler.h`, and the comment at
//! `client/handlers.rs`). So audio and video cannot be decided independently
//! for one request, and a grant is therefore a statement about *a specific
//! combination of devices*, not about "camera" as a standalone permission.
//!
//! That drives the reuse rule in [`MediaGrantStore::covers`]: a stored grant
//! satisfies a later request only if it already covers **every** bit that
//! request asks for. A page previously granted `{audio, video}` may ask again
//! for `{video}` alone and be allowed silently; a page granted `{audio}` that
//! now also wants video is a genuinely new question and must re-prompt.
//!
//! # Scope of a grant
//!
//! Keyed by `(block_id, origin)`:
//!
//! - **Pane-scoped.** Two panes showing the same site are two independent trust
//!   decisions; granting in one must not silently grant in the other.
//! - **Origin-scoped**, not per-URL — standard web permission granularity.
//!   Anything finer would re-prompt on every in-site navigation.
//! - **Session-only.** Grants live in memory and die with the process. There is
//!   deliberately no persistence in v1: persisting a camera grant is a
//!   materially different threat model and wants its own decision (spec §3.2).
//!
//! Nothing here decides *policy* — it stores what was decided. Whether a grant
//! may be created at all (prompt, user consent) is the caller's job, and the
//! store is deliberately incapable of inventing one.

use std::collections::HashMap;

/// In-memory record of which origins may capture which devices, per pane.
///
/// Intentionally not `Clone`: a copy could answer "is this granted?" from a
/// snapshot that a revoke has since invalidated, and a stale *allow* is the one
/// error this type must not make possible.
#[derive(Debug, Default)]
pub struct MediaGrantStore {
    /// `(block_id, origin)` → the bitmask that was granted.
    ///
    /// An entry only ever exists because a grant was made; absence is denial.
    grants: HashMap<(String, String), u32>,
}

impl MediaGrantStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Does an existing grant already cover this exact request?
    ///
    /// `true` only when every requested bit is already granted for this
    /// `(pane, origin)`. Callers may then continue the request without
    /// prompting; `false` means ask, and never means deny-outright — the
    /// distinction between "no grant yet" and "user said no" belongs to the
    /// prompt layer, not here.
    ///
    /// An empty request (`requested == 0`) is never covered. There is nothing
    /// to allow, and returning `true` would let a malformed request through a
    /// check that is supposed to be the narrow gate.
    pub fn covers(&self, block_id: &str, origin: &str, requested: u32) -> bool {
        if requested == 0 {
            return false;
        }
        self.grants
            .get(&(block_id.to_string(), origin.to_string()))
            .is_some_and(|granted| granted & requested == requested)
    }

    /// Record that `origin` may capture `bits` in pane `block_id`.
    ///
    /// Replaces any previous grant for that pair rather than accumulating:
    /// a fresh decision supersedes the old one outright, so a user who is
    /// re-prompted after a revoke cannot end up with more than they just
    /// agreed to. Granting `0` removes the entry — "allowed nothing" and
    /// "no grant" are the same state, and keeping a zero entry would be a
    /// second way to spell it.
    pub fn grant(&mut self, block_id: &str, origin: &str, bits: u32) {
        let key = (block_id.to_string(), origin.to_string());
        if bits == 0 {
            self.grants.remove(&key);
        } else {
            self.grants.insert(key, bits);
        }
    }

    /// Drop one origin's grant in one pane.
    ///
    /// Note this only stops *future* requests. It cannot stop a capture already
    /// in flight — CEF exposes no API for that, so revoking a live stream also
    /// requires reloading the pane (spec §3.7). Callers implementing user-facing
    /// revoke must do both; this function alone is not "revoke".
    pub fn revoke(&mut self, block_id: &str, origin: &str) {
        self.grants
            .remove(&(block_id.to_string(), origin.to_string()));
    }

    /// Drop every grant belonging to one pane.
    ///
    /// Must be called when a pane closes. Block ids are not reused in practice,
    /// but leaving entries behind would leak memory for the process lifetime
    /// and — if an id ever were reused — silently hand a new pane the previous
    /// occupant's grants.
    pub fn clear_pane(&mut self, block_id: &str) {
        self.grants.retain(|(pane, _), _| pane != block_id);
    }

    /// Bits currently granted to `origin` in `block_id`, if any. For rendering
    /// an indicator or a revoke affordance; decisions should use [`covers`].
    ///
    /// [`covers`]: Self::covers
    pub fn granted_bits(&self, block_id: &str, origin: &str) -> Option<u32> {
        self.grants
            .get(&(block_id.to_string(), origin.to_string()))
            .copied()
    }

    /// Every `(origin, bits)` pair currently granted in one pane.
    #[allow(dead_code)]
    pub fn pane_grants(&self, block_id: &str) -> Vec<(String, u32)> {
        self.grants
            .iter()
            .filter(|((pane, _), _)| pane == block_id)
            .map(|((_, origin), bits)| (origin.clone(), *bits))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mirrors cef_media_access_permission_types_t; see client/handlers.rs.
    const AUDIO: u32 = 1 << 0;
    const VIDEO: u32 = 1 << 1;
    const DESKTOP_AUDIO: u32 = 1 << 2;

    const PANE: &str = "block-1";
    const ORIGIN: &str = "https://example.com";

    #[test]
    fn absence_is_denial() {
        let s = MediaGrantStore::new();
        assert!(!s.covers(PANE, ORIGIN, VIDEO));
    }

    #[test]
    fn an_exact_grant_covers_the_same_request() {
        let mut s = MediaGrantStore::new();
        s.grant(PANE, ORIGIN, AUDIO | VIDEO);
        assert!(s.covers(PANE, ORIGIN, AUDIO | VIDEO));
    }

    #[test]
    fn a_grant_covers_a_subset_request() {
        // Granted audio+video, page now asks for video alone — already
        // answered, no reason to ask again.
        let mut s = MediaGrantStore::new();
        s.grant(PANE, ORIGIN, AUDIO | VIDEO);
        assert!(s.covers(PANE, ORIGIN, VIDEO));
        assert!(s.covers(PANE, ORIGIN, AUDIO));
    }

    #[test]
    fn a_grant_does_not_cover_a_superset_request() {
        // THE case that matters: audio was allowed, now the page wants the
        // camera too. That is a new question and must re-prompt.
        let mut s = MediaGrantStore::new();
        s.grant(PANE, ORIGIN, AUDIO);
        assert!(!s.covers(PANE, ORIGIN, AUDIO | VIDEO));
        assert!(!s.covers(PANE, ORIGIN, VIDEO));
    }

    #[test]
    fn a_grant_does_not_cover_a_disjoint_request() {
        let mut s = MediaGrantStore::new();
        s.grant(PANE, ORIGIN, AUDIO);
        assert!(!s.covers(PANE, ORIGIN, DESKTOP_AUDIO));
    }

    #[test]
    fn an_empty_request_is_never_covered() {
        let mut s = MediaGrantStore::new();
        s.grant(PANE, ORIGIN, AUDIO | VIDEO);
        assert!(!s.covers(PANE, ORIGIN, 0));
    }

    #[test]
    fn grants_do_not_leak_across_panes() {
        // Two panes on the same site are two independent trust decisions.
        let mut s = MediaGrantStore::new();
        s.grant(PANE, ORIGIN, VIDEO);
        assert!(!s.covers("block-2", ORIGIN, VIDEO));
    }

    #[test]
    fn grants_do_not_leak_across_origins() {
        let mut s = MediaGrantStore::new();
        s.grant(PANE, ORIGIN, VIDEO);
        assert!(!s.covers(PANE, "https://evil.example", VIDEO));
        // Not a prefix match either.
        assert!(!s.covers(PANE, "https://example.com.evil.test", VIDEO));
    }

    #[test]
    fn a_new_grant_replaces_rather_than_accumulates() {
        // Re-granting narrower must NOT leave the old wider bits in place, or a
        // user re-prompted after a revoke would silently keep more than they
        // just agreed to.
        let mut s = MediaGrantStore::new();
        s.grant(PANE, ORIGIN, AUDIO | VIDEO);
        s.grant(PANE, ORIGIN, AUDIO);
        assert!(s.covers(PANE, ORIGIN, AUDIO));
        assert!(!s.covers(PANE, ORIGIN, VIDEO));
    }

    #[test]
    fn granting_zero_clears_the_entry() {
        let mut s = MediaGrantStore::new();
        s.grant(PANE, ORIGIN, VIDEO);
        s.grant(PANE, ORIGIN, 0);
        assert_eq!(s.granted_bits(PANE, ORIGIN), None);
    }

    #[test]
    fn revoke_removes_only_that_pane_and_origin() {
        let mut s = MediaGrantStore::new();
        s.grant(PANE, ORIGIN, VIDEO);
        s.grant(PANE, "https://other.test", VIDEO);
        s.grant("block-2", ORIGIN, VIDEO);

        s.revoke(PANE, ORIGIN);

        assert!(!s.covers(PANE, ORIGIN, VIDEO));
        assert!(s.covers(PANE, "https://other.test", VIDEO));
        assert!(s.covers("block-2", ORIGIN, VIDEO));
    }

    #[test]
    fn clear_pane_removes_every_origin_in_that_pane_only() {
        let mut s = MediaGrantStore::new();
        s.grant(PANE, ORIGIN, VIDEO);
        s.grant(PANE, "https://other.test", AUDIO);
        s.grant("block-2", ORIGIN, VIDEO);

        s.clear_pane(PANE);

        assert!(s.pane_grants(PANE).is_empty());
        assert!(s.covers("block-2", ORIGIN, VIDEO), "other panes untouched");
    }

    #[test]
    fn pane_grants_lists_what_is_held() {
        let mut s = MediaGrantStore::new();
        s.grant(PANE, ORIGIN, AUDIO | VIDEO);
        let listed = s.pane_grants(PANE);
        assert_eq!(listed, vec![(ORIGIN.to_string(), AUDIO | VIDEO)]);
    }
}
