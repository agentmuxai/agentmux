// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::super::fresh_start_needs_disclosure;

/// The case STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md
/// recorded: a pane's first spawn, no registry pointer to resume, prior
/// history on disk. Nothing else in the resume machinery reports this.
#[test]
fn a_first_spawn_with_no_resume_is_disclosed() {
    assert!(fresh_start_needs_disclosure(None, 1));
}

/// A resume WAS attempted — `persistent_resume`'s own tracking owns the
/// outcome from here (Resumed, or Fresh via the retry/recovery cascade).
/// Disclosing here too would double-report and could contradict it.
#[test]
fn a_spawn_that_attempted_a_resume_is_never_disclosed_here() {
    assert!(!fresh_start_needs_disclosure(Some("some-sid"), 1));
    assert!(!fresh_start_needs_disclosure(Some("some-sid"), 4));
}

/// Later generations spawn without `--resume` too, but each already has
/// its own disclosure or deliberately has none —
/// `retry_after_resume_failure` emits `Fresh` itself when no recovery
/// candidate exists, and `respawn_once_for_leftover_queue` restarts a
/// session already decided on an earlier generation. Re-disclosing would
/// stack a second divider onto an unchanged conversation.
#[test]
fn a_later_generation_respawn_is_not_re_disclosed() {
    assert!(!fresh_start_needs_disclosure(None, 2));
    assert!(!fresh_start_needs_disclosure(None, 17));
}

/// Generation 0 never reaches a spawn (`spawn_process` bumps before use),
/// but the gate must not treat the sentinel as a first spawn.
#[test]
fn generation_zero_is_not_treated_as_a_first_spawn() {
    assert!(!fresh_start_needs_disclosure(None, 0));
}
