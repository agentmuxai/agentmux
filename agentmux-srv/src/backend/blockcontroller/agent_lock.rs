// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-memory registry of PtyShell agent leases — the authoritative, hot-path
//! answer to "is a human's keystroke for this block currently locked out?".
//!
//! # Why this exists as its own thing rather than a meta read
//!
//! The lease (`term:agentlockuntil`,
//! `docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md` §10.3) is
//! enforced in `controllerinput` (`server/websocket.rs`). That handler
//! serves the Stop button's `SIGINT` (`useAgentCommands.ts`) and the
//! debounced PTY-width resizes `usePtyWidth.ts` sends — the latter in bursts
//! while a pane is actively being dragged, which is precisely when the UI is
//! most latency-sensitive. (It is NOT the keystroke path: terminal
//! keystrokes go through the `blockinput` WS command instead —
//! `termViewModel.ts`, `AgentShellSubblock.tsx` — see the gap note at the
//! bottom of this comment.) The first implementation answered the
//! is-it-leased question by reading the block out of `Store`:
//!
//! ```ignore
//! wstore.get::<Block>(&cmd.blockid)  // synchronous SQLite, on every keystroke
//! ```
//!
//! `Store` is one process-wide SQLite connection behind one `Mutex<Connection>`
//! (`storage/store.rs`'s own doc comment: "matching Go's `MaxOpenConns(1)`"),
//! and `Store::get` is a plain blocking call — so that read put a real disk
//! query, serialized against every other `Store` user in the process, inside
//! an async WS handler, inline on a Tokio worker thread with no
//! `block_in_place`/`spawn_blocking`. That is the same failure class the
//! sysinfo incident already cost this repo once (commit `0f34704a8`, #1782).
//! See `docs/analysis/ANALYSIS_CROSS_PANE_INPUT_DELAY_REGRESSION_2026_09_15.md`.
//!
//! The lease is short-lived (`AGENT_LOCK_WINDOW_MS`, 4s), process-local, and
//! recreated on every agent write. None of that needs durability — so it
//! lives here instead: a `HashMap` behind an uncontended `Mutex`, held for
//! nanoseconds with no I/O inside it. For the overwhelmingly common case
//! (no agent is driving any shell) the map is empty and the check is a hash
//! lookup that misses.
//!
//! # Known gap this does NOT close
//!
//! `controllerinput` is not the path a human's keystrokes actually take.
//! Both the Terminal pane (`termViewModel.ts`) and the agent pane's drawer
//! shell (`AgentShellSubblock.tsx`) send keystrokes via the `blockinput` WS
//! command, which calls `blockcontroller::send_input` directly with no lease
//! check at all. So the server-side enforcement added in #3194 does not in
//! fact cover the human-vs-agent write collision it was added for — the only
//! thing standing in the way there is `AgentShellSubblock`'s own
//! `agentLocked()` gate, i.e. exactly the eventually-consistent frontend
//! check that enforcement was meant to backstop. Deliberately left alone
//! here: this module changes how the existing check is answered, not which
//! paths are checked. Closing the gap means calling `is_locked` from the
//! `blockinput` arm too (cheap, now that answering costs a hash lookup) —
//! tracked separately so it lands with its own test and its own review.
//!
//! # Relationship to `term:agentlockuntil` meta
//!
//! The meta key is still written, and is still what `AgentShellSubblock.tsx`
//! renders its "Agent is using this shell" badge from — but it is now the UX
//! layer only, exactly as that handler's own comment already described it
//! ("the frontend gate is the UX layer that keeps it from happening in the
//! first place, not the correctness boundary"). This registry is the
//! correctness boundary. They're written together (`lock_shell_for_agent`)
//! and cleared together (`handle_pty_shell_stop`, and shell exit — below).
//!
//! # Fail-open on restart, deliberately
//!
//! A restarted srv starts with an empty registry, so no lease survives it.
//! That is the right direction to fail for a lock whose entire purpose is a
//! UX-level write-collision guard, not a security boundary: the failure mode
//! of a stale lease is *a human who cannot type in their own terminal* — at
//! worst, cannot type `exit` to close the pane — which is strictly worse than
//! an agent's write interleaving with a human's. Persisted meta could
//! previously outlive the process that set it; memory cannot.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// block_id → absolute unix-ms timestamp the lease expires at.
static LEASES: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();

fn leases() -> &'static Mutex<HashMap<String, i64>> {
    LEASES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Lease `block_id` against human input until `until_ms` (absolute unix ms).
///
/// Called before every agent write (`PtyShellInput`/`PtyShellResize`), which
/// is what makes the lease self-renewing while an agent is actively working
/// and self-releasing ~`AGENT_LOCK_WINDOW_MS` after it stops. A lease that is
/// already in the past is not stored (and clears any existing one) — there is
/// no such thing as a lease that is expired the moment it's taken, and
/// keeping it would only leave an entry for `is_locked` to prune later.
pub fn lock_until(block_id: &str, until_ms: i64) {
    let mut map = leases().lock().unwrap();
    if until_ms <= agentmux_common::time::now_ms() {
        map.remove(block_id);
        return;
    }
    map.insert(block_id.to_string(), until_ms);
}

/// Is a human's input for `block_id` currently locked out by an agent lease?
///
/// The hot path — see the module doc. Prunes the entry when it finds an
/// expired one, so the map stays bounded by the number of *currently* leased
/// blocks (in practice: the number of agents driving a shell right now,
/// normally zero) rather than by every block ever leased.
pub fn is_locked(block_id: &str) -> bool {
    let mut map = leases().lock().unwrap();
    match map.get(block_id) {
        None => false,
        Some(&until_ms) => {
            if until_ms > agentmux_common::time::now_ms() {
                true
            } else {
                map.remove(block_id);
                false
            }
        }
    }
}

/// Release `block_id`'s lease immediately. Returns whether it was actually
/// held (and unexpired) at the moment of the call — `PtyShellStop` reports
/// that back to the agent as `released`.
///
/// Called from `PtyShellStop` (the explicit release) and from a shell's
/// exit path (`shell/lifecycle.rs`) — an exited shell must never keep a
/// lease alive, or the pane it belongs to would refuse the human's keystrokes
/// for up to the rest of the window for a PTY that isn't there any more.
pub fn release(block_id: &str) -> bool {
    let mut map = leases().lock().unwrap();
    match map.remove(block_id) {
        Some(until_ms) => until_ms > agentmux_common::time::now_ms(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Global state shared across the whole test binary — every test here
    /// uses its own block ids so parallel tests can't observe each other.
    fn id(suffix: &str) -> String {
        format!("agent-lock-test-{suffix}")
    }

    fn entry_count(block_id: &str) -> usize {
        leases().lock().unwrap().keys().filter(|k| *k == block_id).count()
    }

    #[test]
    fn an_unleased_block_is_never_locked() {
        assert!(!is_locked(&id("never-leased")));
    }

    #[test]
    fn a_future_lease_locks_and_an_expired_one_does_not() {
        let b = id("future-then-expired");
        lock_until(&b, agentmux_common::time::now_ms() + 60_000);
        assert!(is_locked(&b), "a lease ending in the future must lock");

        lock_until(&b, agentmux_common::time::now_ms() - 1);
        assert!(!is_locked(&b), "a lease ending in the past must not lock");
    }

    /// The expiry must be evaluated at read time, not stored as a boolean —
    /// otherwise a lease taken while an agent worked would still read as
    /// locked minutes later, which is exactly the "human can't type `exit`"
    /// failure this whole mechanism has to avoid.
    #[test]
    fn a_lease_that_lapses_stops_locking_without_anyone_releasing_it() {
        let b = id("lapses-on-its-own");
        let now = agentmux_common::time::now_ms();
        lock_until(&b, now + 40);
        assert!(is_locked(&b));
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(!is_locked(&b), "a lapsed lease must stop locking on its own");
    }

    #[test]
    fn reading_a_lapsed_lease_prunes_it() {
        let b = id("pruned-on-read");
        lock_until(&b, agentmux_common::time::now_ms() + 20);
        assert_eq!(entry_count(&b), 1);
        std::thread::sleep(std::time::Duration::from_millis(40));
        assert!(!is_locked(&b));
        assert_eq!(entry_count(&b), 0, "a lapsed lease must not be retained after it's read");
    }

    /// `lock_until` with an already-past timestamp is a caller bug, but it
    /// must not leave an entry behind for `is_locked` to trip over later.
    #[test]
    fn locking_into_the_past_stores_nothing() {
        let b = id("past-lease");
        lock_until(&b, agentmux_common::time::now_ms() - 5_000);
        assert_eq!(entry_count(&b), 0);
        assert!(!is_locked(&b));
    }

    #[test]
    fn release_reports_whether_a_live_lease_was_held() {
        let b = id("release-reporting");
        assert!(!release(&b), "releasing an unleased block reports nothing was held");

        lock_until(&b, agentmux_common::time::now_ms() + 60_000);
        assert!(release(&b), "releasing a live lease reports it was held");
        assert!(!is_locked(&b), "release must take effect immediately");
        assert_eq!(entry_count(&b), 0);

        lock_until(&b, agentmux_common::time::now_ms() + 30);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!release(&b), "releasing an already-lapsed lease reports nothing was held");
    }

    /// One agent leasing its own pane's shell must not lock anyone else out —
    /// the lease is per-block, and the registry is shared.
    #[test]
    fn leases_are_independent_per_block() {
        let leased = id("independent-leased");
        let other = id("independent-other");
        lock_until(&leased, agentmux_common::time::now_ms() + 60_000);
        assert!(is_locked(&leased));
        assert!(!is_locked(&other));
        release(&leased);
    }

    /// Re-leasing while a lease is live extends it rather than stacking —
    /// this is what makes "refreshed on every agent write" work.
    #[test]
    fn relocking_extends_the_same_entry() {
        let b = id("extend");
        lock_until(&b, agentmux_common::time::now_ms() + 30);
        lock_until(&b, agentmux_common::time::now_ms() + 60_000);
        assert_eq!(entry_count(&b), 1);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(is_locked(&b), "the later lease must win, not the earlier expiry");
        release(&b);
    }
}
