// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Process-wide caps on simultaneous ambient Haiku CLI spawns, one per class of
//! caller. `AmbientGateway` coalesces and cancels only within one
//! `(entity, purpose)` key; without these, N panes finishing a turn at the same
//! moment would each spawn their own CLI.

use tokio::sync::Semaphore;

/// Declares `pub fn $name() -> &'static Semaphore` capped at `$cap` permits.
macro_rules! capped {
    ($(#[$meta:meta])* $name:ident, $cap:expr) => {
        $(#[$meta])*
        pub fn $name() -> &'static Semaphore {
            static SEM: std::sync::OnceLock<Semaphore> = std::sync::OnceLock::new();
            SEM.get_or_init(|| Semaphore::new($cap))
        }
    };
}

/// Max simultaneous Haiku CLI spawns across the two user-turn-triggered pull
/// RPCs (`session:activity_summary`, `session:next_prompt_suggestion`). The
/// Ambient Model Call gateway's `admit()` only dedupes/cancels *per key*
/// (per block+purpose) — with no cap across different blocks, many panes
/// finishing a turn around the same moment could each spawn their own Haiku
/// subprocess unbounded. Mirrors `activity_watcher.rs`'s
/// `MAX_CONCURRENT_SUMMARIES` cap on the separate pushed-summary sweep, but
/// kept as its own semaphore rather than shared with that one: a burst of
/// background sweep summaries should not queue behind, or block, a live
/// user-facing pane-header/ghost-text request, and vice versa.
pub const MAX_CONCURRENT_PULL_CALLS: usize = 2;

capped!(pull_call_semaphore, MAX_CONCURRENT_PULL_CALLS);

/// Max simultaneous Haiku CLI spawns for definition-summary generation.
/// Deliberately its OWN semaphore, not `pull_call_semaphore()` (reagent P1,
/// PR #2786): that one is reserved for live, user-turn-triggered pull RPCs
/// specifically so background bursts don't queue behind or block them (see
/// its own doc comment above) — this call is background-triggered (a
/// `listrecentsessions` poll, not a direct user action), the same class as
/// `activity_watcher.rs`'s pushed-summary sweep, which likewise gets its
/// own dedicated semaphore rather than sharing this one. Capped at 1: this
/// is best-effort background fill-in, not latency-sensitive.
const MAX_CONCURRENT_DEFINITION_SUMMARIES: usize = 1;

capped!(definition_summary_semaphore, MAX_CONCURRENT_DEFINITION_SUMMARIES);

/// Max simultaneous Haiku CLI spawns for `SubagentWatcher::resolve_unnamed_backlog`'s
/// bounded backfill-naming pass. Deliberately its OWN semaphore, not
/// `pull_call_semaphore()` — same reasoning as `definition_summary_semaphore()`
/// above: this is a background burst triggered by a Swarm-pane-open, not a
/// live user-turn-triggered call (`subagent.GenerateName`'s on-click path
/// still uses `pull_call_semaphore()` directly), so it must not queue behind
/// or block that one. Capped at 1: this is best-effort backlog fill-in for
/// historical rows, not latency-sensitive — see
/// docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md for why an
/// unbounded version of this exact call pattern is the incident this is
/// designed not to repeat.
const MAX_CONCURRENT_BACKLOG_NAMING: usize = 1;

capped!(backlog_naming_semaphore, MAX_CONCURRENT_BACKLOG_NAMING);

/// Max simultaneous Haiku CLI spawns for narration, across ALL blocks.
///
/// Load-bearing, and not inherited from anywhere: `AmbientGateway` deduplicates
/// and cancels only within a single `(entity_id, purpose)` key, so without this
/// N panes backgrounding a task at the same moment means N concurrent CLI
/// spawns. The pushed-summary caller `tasks::generate_ambient_narration` is modelled on is bounded by
/// a semaphore that lives in its *caller* (`activity_watcher.rs`), which an
/// RPC-driven path inherits nothing from. The 15s timeout inside
/// `invoke_haiku` bounds each call's duration, not how many run at
/// once. (codex P2 on #3161.)
const MAX_CONCURRENT_NARRATIONS: usize = 2;

capped!(narration_semaphore, MAX_CONCURRENT_NARRATIONS);

#[cfg(test)]
mod pull_call_semaphore_tests {
    use super::*;

    /// The two pull RPCs share one process-wide cap: once
    /// MAX_CONCURRENT_PULL_CALLS permits are held, a further non-blocking
    /// acquire must fail rather than let a third Haiku CLI spawn through.
    #[tokio::test]
    async fn caps_at_max_concurrent_pull_calls() {
        let sem = pull_call_semaphore();
        let mut held = Vec::new();
        for _ in 0..MAX_CONCURRENT_PULL_CALLS {
            held.push(sem.try_acquire().expect("permit within the cap should be available"));
        }
        assert!(
            sem.try_acquire().is_err(),
            "a permit beyond MAX_CONCURRENT_PULL_CALLS must not be granted"
        );

        // Releasing one frees a slot for the next caller.
        held.pop();
        assert!(sem.try_acquire().is_ok(), "releasing a permit must free a slot");
    }
}
