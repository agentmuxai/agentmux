// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One spawn attempt's stale-`--resume` retry decision, plus any terminal
//! error-result line observed for it, as a single explicit state machine.
//!
//! This replaces three fields that used to live directly on
//! `persistent::PersistentInner` — `pending_resume_retry`,
//! `confirmed_stale_resume_retry`, and `pending_error_result_line` — which
//! were mutated directly by four independently-scheduled tokio tasks
//! (stdin writer, stdout reader, stderr reader, process-waiter) racing on
//! a shared mutex. That shape caused the exact bug this module exists to
//! prevent, twice: issue #2368's original report, and a live-reproduced
//! recurrence (agent "Marks", 2026-07-30) where the stderr reader's
//! `poison_resume` promotion landed ~500ms before the stdout reader
//! finished draining lines, and a check of only one of the two fields
//! missed the stash entirely.
//!
//! `update()` is a pure `(state, event) -> (state, effects)` function —
//! every transition is exhaustively unit-testable without a real child
//! process, a real tokio task, or any timing assumptions. Callers
//! (`persistent.rs`'s I/O tasks) are responsible for actually executing
//! the returned `ResumeEffect`s (persisting/dropping/flushing a line,
//! firing the retry, publishing terminal status) — this module only
//! decides, never performs I/O.
//!
//! Every event that could affect in-flight state carries the exact
//! `generation` it was observed for. `update()` ignores an event whose
//! generation doesn't match the state's currently-tracked generation, so
//! a stale event from an already-resolved generation can never corrupt a
//! later, unrelated generation's state — mirrors
//! `persistent.rs`'s own `stop_requested_generation` doc comment
//! ("monotonic generation numbers are never reused, so an unconsumed
//! stale value here is inert, not a leak").

use super::persistent::PersistentSpawnConfig;

/// The spawn config + the growing batch of stdin messages to redeliver if
/// this generation's `--resume` turns out to be stale. Named separately
/// from the state enum so `MessageAppendedToRetryBatch` can grow it in
/// place without reconstructing the whole variant.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RetryPayload {
    pub config: PersistentSpawnConfig,
    pub messages: Vec<String>,
}

/// One spawn attempt's resume/error-line lifecycle.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) enum ResumeState {
    /// No `--resume` attempt is in flight for the current generation —
    /// either this spawn never attached `--resume`, or a prior attempt
    /// already resolved (session id captured, retry fired, or exited
    /// with an unrelated error).
    #[default]
    NotTracking,
    /// `--resume <attempted_sid>` was attached at spawn; outcome not yet
    /// known. `held_error_line` is `Some` once a terminal
    /// `result`/`is_error:true` stdout line has arrived but its
    /// persistence is still undecided. `stop_requested` records an
    /// explicit user Stop that must override a retry even if one gets
    /// confirmed later.
    AwaitingOutcome {
        generation: u64,
        attempted_sid: String,
        retry: RetryPayload,
        held_error_line: Option<String>,
        stop_requested: bool,
    },
    /// The stderr reader confirmed `attempted_sid` unreachable under the
    /// current config dir — the retry WILL fire the moment the process
    /// actually exits, unless `stop_requested` overrides it.
    ConfirmedRetry {
        generation: u64,
        retry: RetryPayload,
        held_error_line: Option<String>,
        stop_requested: bool,
    },
}

impl ResumeState {
    /// Read-only accessor for the drain: is `delivered` the exact message
    /// `spawn_process` originally seeded the retry batch with (its first
    /// entry), for the given `generation`? Used to identify the
    /// already-recorded seed by content (not position) before appending
    /// any LATER message the drain delivers — see
    /// `PersistentSubprocessController::drain_queue_after_successful_spawn`'s
    /// own doc comment for why content, not position, is what's checked.
    pub(super) fn is_seeded_message(&self, generation: u64, delivered: &str) -> bool {
        match self {
            ResumeState::AwaitingOutcome { generation: g, retry, .. }
            | ResumeState::ConfirmedRetry { generation: g, retry, .. }
                if *g == generation =>
            {
                retry.messages.first().map(String::as_str) == Some(delivered)
            }
            _ => false,
        }
    }
}

/// Events the stdin writer / stdout reader / stderr reader / process-
/// waiter sensor tasks report. Each carries the exact spawn `generation`
/// it was observed for.
#[derive(Debug, Clone)]
pub(super) enum ResumeEvent {
    /// A fresh spawn attached `--resume <attempted_sid>`; `generation` is
    /// this spawn's identity for the rest of its lifetime.
    SpawnedWithResume { generation: u64, attempted_sid: String, retry: RetryPayload },
    /// A fresh spawn did NOT attach `--resume` (no session id held yet).
    /// Carries no generation — a fresh spawn always resets tracking to
    /// `NotTracking` regardless of which generation it is.
    SpawnedFresh,
    /// A session-id-bearing stdout frame arrived for this generation.
    /// `sid` is the id it carried; `is_confirmed_success` is true only
    /// when THIS exact frame is a terminal `result` with `is_error:false`
    /// — a fully-completed, genuinely successful turn.
    ///
    /// reagentx P0 on PR #2371: the CLI echoes back whatever `--resume`
    /// sid it was given as its FIRST stdout line REGARDLESS of whether
    /// that resume goes on to succeed or fail — this is true even for a
    /// frame that isn't itself an error (e.g. a "system"/init frame), and
    /// this first echo can arrive before the independently-scheduled
    /// stderr reader has had a chance to report "No conversation found"
    /// and poison it. So `sid == attempted_sid` alone is NOT proof of
    /// genuine progress — it's ambiguous until EITHER a different sid
    /// appears (the CLI gave up and started fresh — unambiguous) OR a
    /// successful terminal result confirms the WHOLE turn actually
    /// completed (also unambiguous). Only those two cases resolve
    /// tracking; a same-sid echo on any other frame type is left
    /// untouched, giving `ResumeUnreachable` a real chance to promote the
    /// retry first if this turns out to be the doomed case.
    SessionCaptured { generation: u64, sid: String, is_confirmed_success: bool },
    /// The drain appended another message to this generation's retry
    /// batch before it was known to be doomed (or after it was
    /// confirmed doomed but before the process actually exited).
    MessageAppendedToRetryBatch { generation: u64, json: String },
    /// stderr reader saw "No conversation found" for this exact sid.
    ResumeUnreachable { generation: u64, sid: String },
    /// stdout reader saw a terminal `result`/`is_error:true` line.
    ErrorResultLine { generation: u64, line: String },
    /// The process actually exited.
    ProcessExited { generation: u64 },
    /// `stop_process` was called while this generation was live.
    StopRequested { generation: u64 },
}

/// What the caller must actually DO in response to an event — kept
/// separate from `ResumeState` so `update()` stays a pure function with
/// no I/O, fully exercisable in a unit test.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ResumeEffect {
    /// Persist `line` to the blockfile immediately — this was never a
    /// resume-retry candidate (fresh spawn, or a generation that already
    /// resolved), so today's behavior (persist as soon as it arrives) is
    /// unchanged.
    PersistImmediately(String),
    /// No retry followed after all (never confirmed, or an explicit
    /// Stop overrode a confirmed one) — flush the held-back line now.
    FlushErrorLine(String),
    /// Fire the retry with this exact payload. `held_error_line` is
    /// bundled here rather than pre-decided (dropped) by this module:
    /// whether the line should actually be dropped depends on whether
    /// the retry ITSELF successfully launches, which this pure function
    /// has no way to know (that's determined later, asynchronously, by
    /// the caller's own spawn attempt). The caller drops it on success
    /// and flushes it on failure — see
    /// `persistent::PersistentSubprocessController::retry_after_resume_failure`.
    FireRetry { retry: RetryPayload, held_error_line: Option<String> },
    /// Genuinely done, not retrying — publish the terminal status.
    PublishDone,
}

/// The pure state transition. See the module doc comment for why this
/// shape exists and what it replaces.
pub(super) fn update(state: ResumeState, event: ResumeEvent) -> (ResumeState, Vec<ResumeEffect>) {
    match (state, event) {
        // A fresh spawn always starts (or restarts) tracking from
        // scratch, regardless of what the previous generation's state
        // was — the previous generation, if any, must have already
        // resolved (its own ProcessExited already ran) before a new
        // spawn_process call can happen on this same controller.
        (_, ResumeEvent::SpawnedWithResume { generation, attempted_sid, retry }) => (
            ResumeState::AwaitingOutcome {
                generation,
                attempted_sid,
                retry,
                held_error_line: None,
                stop_requested: false,
            },
            vec![],
        ),
        (_, ResumeEvent::SpawnedFresh) => (ResumeState::NotTracking, vec![]),

        // A session capture resolves tracking ONLY when it's unambiguous
        // — see `SessionCaptured`'s own doc comment for why a same-sid
        // echo alone isn't proof (the CLI echoes the attempted sid
        // regardless of eventual success or failure). A different sid
        // (the CLI gave up and started fresh) or a genuinely successful
        // terminal result (the whole turn completed) both prove real
        // progress; anything else (the ambiguous early echo, a
        // non-terminal frame) is left untouched so `ResumeUnreachable`
        // still gets a real chance to promote the retry if this turns
        // out to be the doomed case after all.
        (
            ResumeState::AwaitingOutcome { generation, attempted_sid, retry, held_error_line, stop_requested },
            ResumeEvent::SessionCaptured { generation: g, sid, is_confirmed_success },
        ) if generation == g => {
            if sid != attempted_sid || is_confirmed_success {
                // reagentx P2 on PR #2373: a `held_error_line` already
                // stashed (an EARLIER turn on this same still-alive
                // generation held one back, before this LATER capture
                // resolves tracking) must be flushed, not silently
                // discarded along with the state.
                let effects = held_error_line.map(|line| vec![ResumeEffect::FlushErrorLine(line)]).unwrap_or_default();
                (ResumeState::NotTracking, effects)
            } else {
                (
                    ResumeState::AwaitingOutcome { generation, attempted_sid, retry, held_error_line, stop_requested },
                    vec![],
                )
            }
        }
        (ResumeState::ConfirmedRetry { generation, held_error_line, .. }, ResumeEvent::SessionCaptured { generation: g, .. })
            if generation == g =>
        {
            // Defensive: a confirmed retry means the sid is ALREADY known
            // dead (poison_resume already won the race), so this path is
            // not subject to the same first-echo ambiguity — a genuine
            // capture shouldn't happen for this exact generation in
            // practice at all, but if it somehow did, never leave stale
            // tracking state (or a held error line) behind.
            let effects = held_error_line.map(|line| vec![ResumeEffect::FlushErrorLine(line)]).unwrap_or_default();
            (ResumeState::NotTracking, effects)
        }

        // The drain can keep appending messages to the retry batch while
        // still merely pending, or even after already confirmed (up
        // until the doomed process actually exits and the batch is
        // redelivered).
        (
            ResumeState::AwaitingOutcome { generation, attempted_sid, mut retry, held_error_line, stop_requested },
            ResumeEvent::MessageAppendedToRetryBatch { generation: g, json },
        ) if generation == g => {
            retry.messages.push(json);
            (
                ResumeState::AwaitingOutcome { generation, attempted_sid, retry, held_error_line, stop_requested },
                vec![],
            )
        }
        (
            ResumeState::ConfirmedRetry { generation, mut retry, held_error_line, stop_requested },
            ResumeEvent::MessageAppendedToRetryBatch { generation: g, json },
        ) if generation == g => {
            retry.messages.push(json);
            (ResumeState::ConfirmedRetry { generation, retry, held_error_line, stop_requested }, vec![])
        }

        // Promotion: only when the poisoned sid is the EXACT one this
        // generation attempted — an unrelated/mismatched sid leaves the
        // state untouched (falls through to the catch-all below).
        (
            ResumeState::AwaitingOutcome { generation, attempted_sid, retry, held_error_line, stop_requested },
            ResumeEvent::ResumeUnreachable { generation: g, sid },
        ) if generation == g && attempted_sid == sid => {
            (ResumeState::ConfirmedRetry { generation, retry, held_error_line, stop_requested }, vec![])
        }

        // A terminal error-result line arriving while a resume outcome
        // is still undecided (or already confirmed, but the process
        // hasn't exited yet) is held back — its fate is resolved only
        // once ProcessExited arrives. This is the crux of the whole
        // module: unlike checking two separate mutex fields, ONE state
        // enum makes "is this line still a retry candidate" a single,
        // unambiguous question regardless of which order stderr's
        // confirmation and stdout's error line happen to arrive in.
        //
        // reagentx P2 (round 4 on this PR): `held_error_line` holds
        // exactly one line — a genuinely doomed resume produces at most
        // one `is_error:true` before the retry decision resolves, but
        // persistent mode can accept another user message and get a
        // SECOND error before this generation's resume outcome is ever
        // settled. A second line arriving is unambiguous proof the FIRST
        // is a separate, already-settled turn's error — flush it
        // immediately instead of silently overwriting it.
        (
            ResumeState::AwaitingOutcome { generation, attempted_sid, retry, held_error_line, stop_requested },
            ResumeEvent::ErrorResultLine { generation: g, line },
        ) if generation == g => {
            let effects = held_error_line.map(|old| vec![ResumeEffect::PersistImmediately(old)]).unwrap_or_default();
            (
                ResumeState::AwaitingOutcome {
                    generation,
                    attempted_sid,
                    retry,
                    held_error_line: Some(line),
                    stop_requested,
                },
                effects,
            )
        }
        (
            ResumeState::ConfirmedRetry { generation, retry, held_error_line, stop_requested },
            ResumeEvent::ErrorResultLine { generation: g, line },
        ) if generation == g => {
            let effects = held_error_line.map(|old| vec![ResumeEffect::PersistImmediately(old)]).unwrap_or_default();
            (
                ResumeState::ConfirmedRetry { generation, retry, held_error_line: Some(line), stop_requested },
                effects,
            )
        }
        (ResumeState::NotTracking, ResumeEvent::ErrorResultLine { line, .. }) => {
            (ResumeState::NotTracking, vec![ResumeEffect::PersistImmediately(line)])
        }
        // reagentx P2 on PR #2373: an ErrorResultLine whose generation
        // does NOT match what's currently tracked (a stale/lagging
        // reader's line arriving after a respawn already moved on to a
        // new generation) must still persist — falling through to the
        // generic catch-all below would silently lose it (no effect, no
        // state to hold it in), which a caller checking "were there any
        // effects" can't distinguish from a genuine, correctly-tracked
        // hold-back. Explicit arms here (rather than relying on the
        // catch-all) keep that distinction unambiguous.
        (state @ (ResumeState::AwaitingOutcome { .. } | ResumeState::ConfirmedRetry { .. }), ResumeEvent::ErrorResultLine { line, .. }) => {
            (state, vec![ResumeEffect::PersistImmediately(line)])
        }

        // An explicit Stop must win over a retry even if one gets
        // confirmed later (or was already confirmed) — recorded now,
        // consulted when ProcessExited actually resolves the state.
        (
            ResumeState::AwaitingOutcome { generation, attempted_sid, retry, held_error_line, .. },
            ResumeEvent::StopRequested { generation: g },
        ) if generation == g => (
            ResumeState::AwaitingOutcome {
                generation,
                attempted_sid,
                retry,
                held_error_line,
                stop_requested: true,
            },
            vec![],
        ),
        (
            ResumeState::ConfirmedRetry { generation, retry, held_error_line, .. },
            ResumeEvent::StopRequested { generation: g },
        ) if generation == g => {
            (ResumeState::ConfirmedRetry { generation, retry, held_error_line, stop_requested: true }, vec![])
        }

        // Resolution. This is the only place a retry actually fires or a
        // held-back line's fate is finally decided.
        (
            ResumeState::AwaitingOutcome { generation, held_error_line, .. },
            ResumeEvent::ProcessExited { generation: g },
        ) if generation == g => {
            // Never confirmed (auth failure, rate limit, network blip,
            // or the process simply exited before stderr ever reported
            // anything) — genuinely done, not retrying. A held-back line
            // is a real error the user must still see.
            let mut effects = Vec::new();
            if let Some(line) = held_error_line {
                effects.push(ResumeEffect::FlushErrorLine(line));
            }
            effects.push(ResumeEffect::PublishDone);
            (ResumeState::NotTracking, effects)
        }
        (
            ResumeState::ConfirmedRetry { generation, retry, held_error_line, stop_requested },
            ResumeEvent::ProcessExited { generation: g },
        ) if generation == g => {
            let effects = if stop_requested {
                // The user's explicit Stop wins over a confirmed retry —
                // treat this exactly like "genuinely done, not
                // retrying" instead of silently reviving the agent with
                // the same prompt right after being asked to stop.
                let mut effects = Vec::new();
                if let Some(line) = held_error_line {
                    effects.push(ResumeEffect::FlushErrorLine(line));
                }
                effects.push(ResumeEffect::PublishDone);
                effects
            } else {
                // Whether the held line should actually be dropped
                // depends on whether this retry ITSELF successfully
                // launches — this pure function has no way to know that
                // (it's determined later, asynchronously, by the
                // caller's own spawn attempt), so it's bundled into the
                // effect rather than pre-decided here.
                vec![ResumeEffect::FireRetry { retry, held_error_line }]
            };
            (ResumeState::NotTracking, effects)
        }
        (ResumeState::NotTracking, ResumeEvent::ProcessExited { .. }) => {
            (ResumeState::NotTracking, vec![ResumeEffect::PublishDone])
        }

        // Any other (state, event) pairing is either a mismatched sid
        // (an unrelated tentative retry left untouched by a poison for a
        // different sid) or a stale event whose generation no longer
        // matches what's currently tracked — both are safe no-ops.
        (state, _) => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_config() -> PersistentSpawnConfig {
        PersistentSpawnConfig {
            cli_command: "definitely-not-a-real-binary-xyz".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: std::collections::HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "dead-sid".to_string(),
            message_id: None,
        }
    }

    fn dummy_retry() -> RetryPayload {
        RetryPayload { config: dummy_config(), messages: vec!["{}".to_string()] }
    }

    fn spawned_with_resume(generation: u64) -> ResumeState {
        let (state, effects) = update(
            ResumeState::NotTracking,
            ResumeEvent::SpawnedWithResume {
                generation,
                attempted_sid: "dead-sid".to_string(),
                retry: dummy_retry(),
            },
        );
        assert!(effects.is_empty(), "spawning never produces an effect on its own");
        state
    }

    #[test]
    fn fresh_spawn_persists_error_lines_immediately() {
        let (state, effects) = update(
            ResumeState::NotTracking,
            ResumeEvent::ErrorResultLine { generation: 1, line: "boom".to_string() },
        );
        assert_eq!(state, ResumeState::NotTracking);
        assert_eq!(effects, vec![ResumeEffect::PersistImmediately("boom".to_string())]);
    }

    #[test]
    fn fresh_spawn_exit_just_publishes_done() {
        let (state, effects) = update(ResumeState::NotTracking, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking);
        assert_eq!(effects, vec![ResumeEffect::PublishDone]);
    }

    #[test]
    fn successful_session_capture_stands_down_tracking() {
        let state = spawned_with_resume(1);
        let (state, effects) = update(
            state,
            ResumeEvent::SessionCaptured {
                generation: 1,
                sid: "dead-sid".to_string(),
                is_confirmed_success: true,
            },
        );
        assert_eq!(state, ResumeState::NotTracking);
        assert!(effects.is_empty());

        // A later, unrelated exit on this now-resolved generation must
        // not retry or dredge up anything.
        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking);
        assert_eq!(effects, vec![ResumeEffect::PublishDone]);
    }

    // reagentx P0 on PR #2373: the CLI echoes the attempted sid on its
    // FIRST stdout line regardless of whether the resume goes on to
    // succeed or fail (e.g. a "system"/init frame) — so a same-sid
    // capture that is NOT a confirmed success must leave tracking
    // untouched, giving `ResumeUnreachable` a real chance to promote the
    // retry afterward if this turns out to be the doomed case.
    #[test]
    fn ambiguous_same_sid_echo_that_is_not_a_confirmed_success_leaves_tracking_untouched() {
        let state = spawned_with_resume(1);
        let (state, effects) = update(
            state,
            ResumeEvent::SessionCaptured {
                generation: 1,
                sid: "dead-sid".to_string(),
                is_confirmed_success: false,
            },
        );
        assert!(
            matches!(&state, ResumeState::AwaitingOutcome { .. }),
            "an unconfirmed same-sid echo must not resolve tracking"
        );
        assert!(effects.is_empty());

        // The retry still gets a real chance to be promoted afterward.
        let (state, _) =
            update(state, ResumeEvent::ResumeUnreachable { generation: 1, sid: "dead-sid".to_string() });
        assert!(matches!(state, ResumeState::ConfirmedRetry { .. }));
    }

    #[test]
    fn error_line_before_poison_is_held_then_bundled_into_the_firing_retry() {
        // Reproduces the ORIGINAL issue #2368 mechanism: the stdout
        // reader's error line arrives before the stderr reader confirms
        // the poison.
        let state = spawned_with_resume(1);
        let (state, effects) =
            update(state, ResumeEvent::ErrorResultLine { generation: 1, line: "boom".to_string() });
        assert!(effects.is_empty(), "held back, not persisted yet");
        assert!(matches!(&state, ResumeState::AwaitingOutcome { held_error_line: Some(l), .. } if l == "boom"));

        let (state, effects) =
            update(state, ResumeEvent::ResumeUnreachable { generation: 1, sid: "dead-sid".to_string() });
        assert!(effects.is_empty(), "promotion alone produces no effect");
        assert!(matches!(&state, ResumeState::ConfirmedRetry { held_error_line: Some(l), .. } if l == "boom"));

        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking);
        assert_eq!(
            effects,
            vec![ResumeEffect::FireRetry { retry: dummy_retry(), held_error_line: Some("boom".to_string()) }]
        );
    }

    // reagentx P2 (round 4 on PR #2373): `held_error_line` holds exactly
    // one line — a genuinely doomed resume produces at most one
    // `is_error:true` before the retry decision resolves, but persistent
    // mode can accept another user message and get a SECOND error before
    // this generation's resume outcome is ever settled. The first line
    // must not be silently lost to the second.
    #[test]
    fn a_second_error_line_supersedes_and_flushes_the_first_instead_of_losing_it() {
        let state = spawned_with_resume(1);
        let (state, effects) =
            update(state, ResumeEvent::ErrorResultLine { generation: 1, line: "first turn's error".to_string() });
        assert!(effects.is_empty(), "nothing was held yet, so nothing to supersede");
        assert!(matches!(&state, ResumeState::AwaitingOutcome { held_error_line: Some(l), .. } if l == "first turn's error"));

        let (state, effects) =
            update(state, ResumeEvent::ErrorResultLine { generation: 1, line: "second turn's error".to_string() });
        assert_eq!(
            effects,
            vec![ResumeEffect::PersistImmediately("first turn's error".to_string())],
            "the first held line must be flushed immediately, not silently discarded"
        );
        assert!(
            matches!(&state, ResumeState::AwaitingOutcome { held_error_line: Some(l), .. } if l == "second turn's error"),
            "the newest line is what's still held, pending this generation's own retry decision"
        );
    }

    #[test]
    fn poison_before_error_line_still_holds_and_bundles_it_into_the_retry() {
        // Reproduces the LIVE-REPRODUCED race (agent "Marks",
        // 2026-07-30): stderr confirms the poison well before the
        // stdout reader ever reaches the doomed line. A design that only
        // checked "is a retry still pending" (not "is one already
        // confirmed") missed this ordering — this state machine doesn't
        // have that gap, because AwaitingOutcome and ConfirmedRetry are
        // the SAME kind of "still tracking, line still undecided" state
        // from ErrorResultLine's point of view.
        let state = spawned_with_resume(1);
        let (state, _) =
            update(state, ResumeEvent::ResumeUnreachable { generation: 1, sid: "dead-sid".to_string() });
        assert!(matches!(state, ResumeState::ConfirmedRetry { .. }));

        let (state, effects) =
            update(state, ResumeEvent::ErrorResultLine { generation: 1, line: "boom".to_string() });
        assert!(effects.is_empty(), "still held back even though already confirmed");
        assert!(matches!(&state, ResumeState::ConfirmedRetry { held_error_line: Some(l), .. } if l == "boom"));

        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking);
        assert_eq!(
            effects,
            vec![ResumeEffect::FireRetry { retry: dummy_retry(), held_error_line: Some("boom".to_string()) }]
        );
    }

    #[test]
    fn unrelated_error_never_confirmed_flushes_instead_of_retrying() {
        // An auth failure / rate limit / network blip on the very first
        // message of a fresh --resume spawn: an error line arrives, but
        // stderr never reports "No conversation found" for it, so no
        // promotion ever happens. The line must still reach the user —
        // just flushed at exit time instead of immediately.
        let state = spawned_with_resume(1);
        let (state, _) =
            update(state, ResumeEvent::ErrorResultLine { generation: 1, line: "auth failed".to_string() });

        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking);
        assert_eq!(
            effects,
            vec![ResumeEffect::FlushErrorLine("auth failed".to_string()), ResumeEffect::PublishDone]
        );
    }

    #[test]
    fn mismatched_sid_does_not_promote() {
        let state = spawned_with_resume(1);
        let (state, effects) =
            update(state, ResumeEvent::ResumeUnreachable { generation: 1, sid: "some-other-sid".to_string() });
        assert!(effects.is_empty());
        assert!(
            matches!(state, ResumeState::AwaitingOutcome { .. }),
            "a poison for an unrelated sid must not promote this generation's tentative retry"
        );
    }

    #[test]
    fn stale_generation_event_is_ignored() {
        // Generation 1 already resolved; a late-arriving event still
        // tagged with generation 1 must not corrupt generation 2's fresh
        // state.
        let state = spawned_with_resume(1);
        let (state, _) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking);

        let state = spawned_with_resume(2);
        let (state, effects) =
            update(state, ResumeEvent::ResumeUnreachable { generation: 1, sid: "dead-sid".to_string() });
        assert!(effects.is_empty());
        assert!(
            matches!(state, ResumeState::AwaitingOutcome { generation: 2, .. }),
            "a stale generation-1 event must not affect generation 2's state"
        );
    }

    #[test]
    fn message_appended_while_awaiting_outcome_grows_the_batch() {
        let state = spawned_with_resume(1);
        let (state, effects) =
            update(state, ResumeEvent::MessageAppendedToRetryBatch { generation: 1, json: "{\"m\":2}".to_string() });
        assert!(effects.is_empty());
        match state {
            ResumeState::AwaitingOutcome { retry, .. } => {
                assert_eq!(retry.messages, vec!["{}".to_string(), "{\"m\":2}".to_string()])
            }
            other => panic!("expected AwaitingOutcome, got {other:?}"),
        }
    }

    #[test]
    fn message_appended_after_confirmed_still_grows_the_batch() {
        // Matches persistent.rs's existing
        // `drain_appends_to_confirmed_retry_once_already_promoted_from_pending`
        // — the drain can still be delivering messages after the retry
        // is confirmed but before the doomed process has actually
        // exited.
        let state = spawned_with_resume(1);
        let (state, _) =
            update(state, ResumeEvent::ResumeUnreachable { generation: 1, sid: "dead-sid".to_string() });
        let (state, effects) =
            update(state, ResumeEvent::MessageAppendedToRetryBatch { generation: 1, json: "{\"m\":2}".to_string() });
        assert!(effects.is_empty());
        match state {
            ResumeState::ConfirmedRetry { retry, .. } => {
                assert_eq!(retry.messages, vec!["{}".to_string(), "{\"m\":2}".to_string()])
            }
            other => panic!("expected ConfirmedRetry, got {other:?}"),
        }
    }

    #[test]
    fn stop_requested_before_confirmation_overrides_a_later_confirmed_retry() {
        let state = spawned_with_resume(1);
        let (state, _) = update(state, ResumeEvent::StopRequested { generation: 1 });
        let (state, _) =
            update(state, ResumeEvent::ResumeUnreachable { generation: 1, sid: "dead-sid".to_string() });
        assert!(matches!(state, ResumeState::ConfirmedRetry { stop_requested: true, .. }));

        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking);
        assert_eq!(
            effects,
            vec![ResumeEffect::PublishDone],
            "an explicit stop must win over a confirmed retry, not silently revive the agent"
        );
    }

    #[test]
    fn stop_requested_after_confirmation_still_overrides_the_retry() {
        let state = spawned_with_resume(1);
        let (state, _) =
            update(state, ResumeEvent::ResumeUnreachable { generation: 1, sid: "dead-sid".to_string() });
        let (state, _) = update(state, ResumeEvent::StopRequested { generation: 1 });
        assert!(matches!(state, ResumeState::ConfirmedRetry { stop_requested: true, .. }));

        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking);
        assert_eq!(effects, vec![ResumeEffect::PublishDone]);
    }

    #[test]
    fn stop_requested_with_a_held_error_line_still_flushes_it() {
        // A stop overriding a confirmed retry still owes the user
        // whatever error was actually observed, same as the
        // never-confirmed case.
        let state = spawned_with_resume(1);
        let (state, _) =
            update(state, ResumeEvent::ErrorResultLine { generation: 1, line: "boom".to_string() });
        let (state, _) =
            update(state, ResumeEvent::ResumeUnreachable { generation: 1, sid: "dead-sid".to_string() });
        let (state, _) = update(state, ResumeEvent::StopRequested { generation: 1 });

        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking);
        assert_eq!(
            effects,
            vec![ResumeEffect::FlushErrorLine("boom".to_string()), ResumeEffect::PublishDone]
        );
    }

    // reagentx P2 on PR #2373: a `held_error_line` stashed by an EARLIER
    // turn on this same still-alive generation must be flushed, not
    // silently discarded, when a LATER turn's session-id capture
    // resolves tracking before ProcessExited ever arrives.
    #[test]
    fn session_captured_flushes_a_held_error_line_from_an_earlier_turn() {
        let state = spawned_with_resume(1);
        let (state, _) =
            update(state, ResumeEvent::ErrorResultLine { generation: 1, line: "boom".to_string() });
        assert!(matches!(&state, ResumeState::AwaitingOutcome { held_error_line: Some(_), .. }));

        let (state, effects) = update(
            state,
            ResumeEvent::SessionCaptured { generation: 1, sid: "dead-sid".to_string(), is_confirmed_success: true },
        );
        assert_eq!(state, ResumeState::NotTracking);
        assert_eq!(
            effects,
            vec![ResumeEffect::FlushErrorLine("boom".to_string())],
            "the held error must reach the user, not vanish along with the resolved tracking"
        );
    }

    // Same as above, but the retry was ALREADY confirmed (defensive path
    // — a genuine capture shouldn't happen for a poisoned sid in
    // practice, but must not silently lose a held line if it somehow
    // does).
    #[test]
    fn session_captured_flushes_a_held_error_line_even_after_confirmation() {
        let state = spawned_with_resume(1);
        let (state, _) =
            update(state, ResumeEvent::ErrorResultLine { generation: 1, line: "boom".to_string() });
        let (state, _) =
            update(state, ResumeEvent::ResumeUnreachable { generation: 1, sid: "dead-sid".to_string() });
        assert!(matches!(&state, ResumeState::ConfirmedRetry { held_error_line: Some(_), .. }));

        let (state, effects) = update(
            state,
            ResumeEvent::SessionCaptured { generation: 1, sid: "dead-sid".to_string(), is_confirmed_success: false },
        );
        assert_eq!(state, ResumeState::NotTracking);
        assert_eq!(effects, vec![ResumeEffect::FlushErrorLine("boom".to_string())]);
    }

    // reagentx P2 on PR #2373: an ErrorResultLine whose generation
    // doesn't match what's currently tracked (a stale/lagging reader's
    // line arriving after a respawn already moved on) must still persist
    // — a caller checking "were there any effects" to decide hold-back
    // vs. not can't otherwise distinguish this from a genuine,
    // correctly-tracked hold-back, silently losing the line.
    #[test]
    fn mismatched_generation_error_line_persists_instead_of_vanishing() {
        let state = spawned_with_resume(2);
        let (state, effects) =
            update(state, ResumeEvent::ErrorResultLine { generation: 1, line: "orphaned".to_string() });
        assert_eq!(
            effects,
            vec![ResumeEffect::PersistImmediately("orphaned".to_string())],
            "a line tagged with a generation that no longer matches must still reach the user"
        );
        // The mismatched event must not disturb generation 2's own state.
        assert!(matches!(state, ResumeState::AwaitingOutcome { generation: 2, held_error_line: None, .. }));
    }

    #[test]
    fn spawned_fresh_with_no_resume_never_tracks_anything() {
        let (state, effects) = update(ResumeState::NotTracking, ResumeEvent::SpawnedFresh);
        assert_eq!(state, ResumeState::NotTracking);
        assert!(effects.is_empty());

        let (state, effects) =
            update(state, ResumeEvent::ErrorResultLine { generation: 1, line: "boom".to_string() });
        assert_eq!(effects, vec![ResumeEffect::PersistImmediately("boom".to_string())]);
        let (_, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(effects, vec![ResumeEffect::PublishDone]);
    }
}
