// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Linux (glibc) only: cap glibc's malloc arenas, and pin its mmap threshold.
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
//! The second setting is the mmap threshold. By default glibc raises it (up to 32 MB)
//! every time a mapped block is freed, so after one large buffer has come and gone
//! every later buffer of that size comes from an arena instead of its own mapping.
//! Large, short-lived buffers (a transcript part, a serialized frame) then sit
//! between long-lived allocations, and when they are freed their holes can't be
//! returned to the system: a soak of 12 agents held 43 MB of freed memory beside a
//! live heap of 46 MB. Pinning the threshold at 64 KB keeps those buffers in their own
//! mappings, which go back at `free`. Measured, 12 agents, same build, runs side by
//! side: after 60 rounds srv's private memory was 58.0 MB with this change and 109.6 MB
//! without (CPU +2%); with the threshold set by the environment instead, 57.8 and 59.9 MB
//! against 86.8 and 92.8 MB for the default.
//! docs/specs/SPEC_SRV_MALLOC_ARENA_CAP_2026_10_18.md
//!
//! Windows and macOS have their own allocators, and this does nothing there.

/// How many arenas srv's threads share.
pub const ARENA_MAX: i32 = 2;

/// Blocks of this size and above get their own mapping (glibc's default start is
/// 128 KB, and it grows from there unless the value is set).
pub const MMAP_THRESHOLD: i32 = 64 * 1024;

/// Whether to apply the cap: not when the operator set `MALLOC_ARENA_MAX`
/// (glibc reads it itself, and a deliberate setting wins).
fn should_apply(env_value: Option<&str>) -> bool {
    env_value.map_or(true, str::is_empty)
}

/// Whether to pin the mmap threshold: not when the operator set it, by the
/// variable or through `GLIBC_TUNABLES`.
fn should_pin_mmap_threshold(env_value: Option<&str>, tunables: Option<&str>) -> bool {
    env_value.map_or(true, str::is_empty) && !tunables.is_some_and(|t| t.contains("mmap_threshold"))
}

/// Cap the arenas and pin the mmap threshold. The cap has to run before the first
/// thread is spawned: a thread already attached to an arena keeps it, so this is
/// the first thing `main` does, ahead of building the runtime. Returns whether the
/// cap was set.
pub fn apply() -> bool {
    if should_pin_mmap_threshold(
        std::env::var("MALLOC_MMAP_THRESHOLD_").ok().as_deref(),
        std::env::var("GLIBC_TUNABLES").ok().as_deref(),
    ) {
        // SAFETY: as below. Setting the threshold also stops glibc from raising it.
        unsafe { libc::mallopt(libc::M_MMAP_THRESHOLD, MMAP_THRESHOLD) };
    }
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

    #[test]
    fn the_mmap_threshold_is_pinned_unless_the_operator_set_it() {
        assert!(should_pin_mmap_threshold(None, None));
        assert!(should_pin_mmap_threshold(Some(""), Some("glibc.malloc.arena_max=4")));
        assert!(!should_pin_mmap_threshold(Some("131072"), None));
        assert!(!should_pin_mmap_threshold(None, Some("glibc.malloc.mmap_threshold=1048576")));
    }

    #[test]
    fn the_mmap_threshold_is_below_glibcs_dynamic_start() {
        // glibc starts at 128 KB and only ever raises it; a pinned value above that would change nothing.
        assert!(MMAP_THRESHOLD < 128 * 1024);
    }
}
