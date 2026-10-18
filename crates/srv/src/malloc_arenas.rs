// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Linux (glibc) only: cap glibc's malloc arenas.
//!
//! srv is a multi-threaded process (a tokio worker pool and a pool of blocking
//! threads), and glibc gives threads their own malloc arenas, up to eight per
//! core. Each arena keeps the high-water mark of what its threads allocated and
//! freed, so a long, busy srv holds about three times its live heap in freed
//! memory spread over many arenas. With the cap, threads share two. Measured:
//! 12 tool-heavy agents, srv's private memory settled at a median of 114 MB
//! with the default and 88 MB with two arenas (paired differences 21–32 MB, the
//! same direction in every pair), and no more CPU.
//! docs/specs/SPEC_SRV_MALLOC_ARENA_CAP_2026_10_18.md
//!
//! Windows and macOS have their own allocators, and this does nothing there.

/// How many arenas srv's threads share.
pub const ARENA_MAX: i32 = 2;

/// Whether to apply the cap: not when the operator set `MALLOC_ARENA_MAX`
/// (glibc reads it itself, and a deliberate setting wins).
fn should_apply(env_value: Option<&str>) -> bool {
    env_value.map_or(true, str::is_empty)
}

/// Cap the arenas. It has to run before the first thread is spawned: a thread
/// already attached to an arena keeps it, so this is the first thing `main`
/// does, ahead of building the runtime. Returns whether the cap was set.
pub fn apply() -> bool {
    if !should_apply(std::env::var("MALLOC_ARENA_MAX").ok().as_deref()) {
        return false;
    }
    // SAFETY: `mallopt` only sets a malloc tunable; it is documented as safe to
    // call at any time, and takes effect for arenas created after it.
    unsafe { libc::mallopt(libc::M_ARENA_MAX, ARENA_MAX) == 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_applies_unless_the_operator_set_the_variable() {
        assert!(should_apply(None));
        assert!(should_apply(Some("")), "an empty variable is no setting");
        assert!(!should_apply(Some("1")));
        assert!(!should_apply(Some("8")), "an explicit value wins, whatever it is");
    }

    #[test]
    fn two_arenas_is_the_measured_choice() {
        assert_eq!(ARENA_MAX, 2);
    }
}
