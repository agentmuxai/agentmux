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

/// One redeliverable stdin message in a [`RetryPayload`] batch: the
/// formatted JSON plus its queue identity.
///
/// `seq` is the message's `QueuedMessage::seq` (issue #2365): assigned
/// once from the controller's monotonic counter at first enqueue and
/// preserved across redelivery, so every "is this message already
/// queued / is this the seeded message" check is exact identity, never
/// content equality — two genuinely different messages with identical
/// text (two "yes" replies) can no longer be conflated.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct QueuedRetryEntry {
    pub seq: u64,
    pub json: String,
}

/// The spawn config + the growing batch of stdin messages to redeliver if
/// this generation's `--resume` turns out to be stale. Named separately
/// from the state enum so `MessageAppendedToRetryBatch` can grow it in
/// place without reconstructing the whole variant.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RetryPayload {
    pub config: PersistentSpawnConfig,
    pub messages: Vec<QueuedRetryEntry>,
}

/// One spawn attempt's resume/error-line lifecycle.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ResumeState {
    /// No `--resume` attempt is in flight for the current generation —
    /// either this spawn never attached `--resume`, or a prior attempt
    /// already resolved (session id captured, retry fired, or exited
    /// with an unrelated error). `current_generation` is whichever
    /// generation this resolved FROM (or `0` if no spawn has ever
    /// happened — see `Default` below). reagentx P1 (round 5 on PR
    /// #2373): without this, `NotTracking` couldn't tell a legitimate
    /// `ProcessExited` for the generation that just resolved apart from
    /// a STALE one for an already-superseded older generation, letting
    /// the older one incorrectly publish terminal status over a newer,
    /// actively-running generation. See `ProcessExited`'s handling below.
    NotTracking { current_generation: u64 },
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
    ///
    /// reagentx P1 (round 9 on this PR, also flagged inline by codex):
    /// `attempted_sid` is carried forward from `AwaitingOutcome` (not
    /// dropped at promotion) so `SessionCaptured` can still tell an
    /// ambiguous same-sid echo apart from unambiguous progress even
    /// after confirmation — see that arm's own doc comment for why
    /// treating EVERY capture as unambiguous once confirmed was wrong:
    /// `resume_poisoned` (`persistent.rs`) is a single, non-generation-
    /// scoped, permanent field, so a lagging stderr-reader task from an
    /// OLDER, already-superseded generation can overwrite it to a
    /// different sid while THIS generation is still `ConfirmedRetry`,
    /// silently disabling `try_capture_session_id`'s poison guard for
    /// this generation's own attempted sid and letting a routine
    /// same-sid echo reach this arm.
    ConfirmedRetry {
        generation: u64,
        attempted_sid: String,
        retry: RetryPayload,
        held_error_line: Option<String>,
        stop_requested: bool,
    },
}

impl Default for ResumeState {
    /// `current_generation: 0` — real generations start at 1
    /// (`PersistentInner::spawn_generation` is bumped before its first
    /// use), so this default can never accidentally match a genuine
    /// `ProcessExited` event.
    fn default() -> Self {
        ResumeState::NotTracking { current_generation: 0 }
    }
}

impl ResumeState {
    /// Read-only accessor for the drain: is `delivered_seq` the exact
    /// message `spawn_process` originally seeded the retry batch with
    /// (its first entry), for the given `generation`? Used to identify
    /// the already-recorded seed before appending any LATER message the
    /// drain delivers. Matched by queue identity (`QueuedRetryEntry::seq`),
    /// neither by position (see
    /// `PersistentSubprocessController::drain_queue_after_successful_spawn`'s
    /// own doc comment for why the front of the queue isn't necessarily
    /// the seed) nor by content (issue #2365 — a later, genuinely
    /// different delivery sharing identical text must not be mistaken
    /// for the seed).
    pub(super) fn is_seeded_message(&self, generation: u64, delivered_seq: u64) -> bool {
        match self {
            ResumeState::AwaitingOutcome { generation: g, retry, .. }
            | ResumeState::ConfirmedRetry { generation: g, retry, .. }
                if *g == generation =>
            {
                retry.messages.first().map(|e| e.seq) == Some(delivered_seq)
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
    /// `generation` is this spawn's identity, carried so `NotTracking`
    /// can tell a later, legitimate `ProcessExited` for THIS generation
    /// apart from a stale one for whatever generation it superseded.
    SpawnedFresh { generation: u64 },
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
    MessageAppendedToRetryBatch { generation: u64, entry: QueuedRetryEntry },
    /// stderr reader saw "No conversation found" for this exact sid.
    ResumeUnreachable { generation: u64, sid: String },
    /// stdout reader saw a terminal `result`/`is_error:true` line.
    ErrorResultLine { generation: u64, line: String },
    /// The process actually exited.
    ProcessExited { generation: u64 },
    /// `stop_process` was called while this generation was live.
    StopRequested { generation: u64 },
}

/// A resume attempt's outcome, once definitively known. See
/// `ResumeEffect::EmitSessionOutcome`'s doc comment for how this is used.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum SessionOutcome {
    /// The CLI continued the exact session `--resume` was given.
    Resumed,
    /// The CLI could not continue that session — a new one was (or is
    /// about to be) started. The model has none of the prior turns.
    Fresh,
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
    /// A `--resume <attempted_sid>` attempt's outcome just became
    /// unambiguously known — surface it as a persisted transcript event
    /// (not just a trace log line), so the pane's scrollback can never
    /// silently disagree with what the model actually has in context.
    /// `actual_sid` is `Some` when the CLI reported a different session id
    /// in the same breath that resolved this generation (the
    /// `SessionCaptured` divergence case); `None` for the `FireRetry` path,
    /// where no fresh sid is known yet — the retry hasn't launched.
    /// See SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md §2.1.
    EmitSessionOutcome { outcome: SessionOutcome, attempted_sid: String, actual_sid: Option<String> },
}

/// Resolves a PRIOR generation's still-unresolved tracking
/// (`AwaitingOutcome`/`ConfirmedRetry`) that a fresh spawn is about to
/// supersede.
///
/// reagentx P1 (round 5 on PR #2373): `SpawnedWithResume`/`SpawnedFresh`
/// used to reset straight into the new generation's state with NO
/// effects at all — normally safe, since a new spawn_process call is only
/// supposed to happen after the previous generation's own `ProcessExited`
/// already resolved it, but reachable out of order via
/// `persistent.rs`'s `respawn_once_for_leftover_queue` (a stall-triggered
/// fallback respawn, on a completely different path than this module's
/// own confirmed-retry firing) racing the process-waiter task that would
/// otherwise resolve the OLD generation first. Silently discarding a
/// still-held error line lost a real user-facing error; the retry itself
/// is dropped (not fired) rather than silently discarded-and-lost,
/// because a brand-new spawn is already underway via a completely
/// different code path — firing ANOTHER retry on top of it would
/// double-spawn.
fn resolve_superseded_generation(prior: ResumeState) -> Vec<ResumeEffect> {
    match prior {
        ResumeState::AwaitingOutcome { held_error_line, .. }
        | ResumeState::ConfirmedRetry { held_error_line, .. } => {
            held_error_line.map(|line| vec![ResumeEffect::FlushErrorLine(line)]).unwrap_or_default()
        }
        ResumeState::NotTracking { .. } => vec![],
    }
}

/// The pure state transition. See the module doc comment for why this
/// shape exists and what it replaces.
pub(super) fn update(state: ResumeState, event: ResumeEvent) -> (ResumeState, Vec<ResumeEffect>) {
    match (state, event) {
        // A fresh spawn always starts (or restarts) tracking from
        // scratch — normally the previous generation, if any, has
        // already resolved (its own ProcessExited already ran) before a
        // new spawn_process call happens on this same controller, but
        // `resolve_superseded_generation` closes the gap for the rare
        // out-of-order case (see its own doc comment).
        (prior, ResumeEvent::SpawnedWithResume { generation, attempted_sid, retry }) => {
            let effects = resolve_superseded_generation(prior);
            (
                ResumeState::AwaitingOutcome {
                    generation,
                    attempted_sid,
                    retry,
                    held_error_line: None,
                    stop_requested: false,
                },
                effects,
            )
        }
        (prior, ResumeEvent::SpawnedFresh { generation }) => {
            let effects = resolve_superseded_generation(prior);
            (ResumeState::NotTracking { current_generation: generation }, effects)
        }

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
                let mut effects: Vec<ResumeEffect> =
                    held_error_line.map(ResumeEffect::FlushErrorLine).into_iter().collect();
                // SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md §2.1: this
                // is one of the two points where a resume attempt's fate
                // becomes unambiguously known — surface it.
                let outcome = if sid == attempted_sid && is_confirmed_success {
                    SessionOutcome::Resumed
                } else {
                    SessionOutcome::Fresh
                };
                let actual_sid = if sid != attempted_sid { Some(sid) } else { None };
                effects.push(ResumeEffect::EmitSessionOutcome { outcome, attempted_sid, actual_sid });
                (ResumeState::NotTracking { current_generation: generation }, effects)
            } else {
                (
                    ResumeState::AwaitingOutcome { generation, attempted_sid, retry, held_error_line, stop_requested },
                    vec![],
                )
            }
        }
        // reagentx P1 (round 9 on this PR, also flagged inline by codex):
        // this arm used to resolve UNCONDITIONALLY, reasoning that a
        // confirmed retry means the sid is already known dead so the
        // first-echo ambiguity couldn't apply. That reasoning breaks
        // because `resume_poisoned` (persistent.rs) is a single,
        // non-generation-scoped, PERMANENT field — a lagging stderr-
        // reader task from an OLDER, already-superseded generation can
        // overwrite it to a different sid while THIS generation is
        // still `ConfirmedRetry`, silently disabling
        // `try_capture_session_id`'s poison guard for this generation's
        // own attempted sid. A routine same-sid echo (the CLI sends the
        // attempted sid on every frame, ambiguous or not) can then reach
        // this arm and used to wipe out the confirmed retry payload
        // without ever firing it — silently losing every message the
        // doomed process had already accepted, reproducing the exact
        // #2368 bug class this PR exists to fix. Same unambiguity check
        // as the `AwaitingOutcome` arm above: only a different sid or a
        // confirmed terminal success resolves tracking here too.
        (
            ResumeState::ConfirmedRetry { generation, attempted_sid, retry, held_error_line, stop_requested },
            ResumeEvent::SessionCaptured { generation: g, sid, is_confirmed_success },
        ) if generation == g => {
            if sid != attempted_sid || is_confirmed_success {
                let mut effects: Vec<ResumeEffect> =
                    held_error_line.map(ResumeEffect::FlushErrorLine).into_iter().collect();
                // Same rationale as the `AwaitingOutcome` arm above.
                let outcome = if sid == attempted_sid && is_confirmed_success {
                    SessionOutcome::Resumed
                } else {
                    SessionOutcome::Fresh
                };
                let actual_sid = if sid != attempted_sid { Some(sid) } else { None };
                effects.push(ResumeEffect::EmitSessionOutcome { outcome, attempted_sid, actual_sid });
                (ResumeState::NotTracking { current_generation: generation }, effects)
            } else {
                (
                    ResumeState::ConfirmedRetry { generation, attempted_sid, retry, held_error_line, stop_requested },
                    vec![],
                )
            }
        }

        // The drain can keep appending messages to the retry batch while
        // still merely pending, or even after already confirmed (up
        // until the doomed process actually exits and the batch is
        // redelivered).
        (
            ResumeState::AwaitingOutcome { generation, attempted_sid, mut retry, held_error_line, stop_requested },
            ResumeEvent::MessageAppendedToRetryBatch { generation: g, entry },
        ) if generation == g => {
            retry.messages.push(entry);
            (
                ResumeState::AwaitingOutcome { generation, attempted_sid, retry, held_error_line, stop_requested },
                vec![],
            )
        }
        (
            ResumeState::ConfirmedRetry { generation, attempted_sid, mut retry, held_error_line, stop_requested },
            ResumeEvent::MessageAppendedToRetryBatch { generation: g, entry },
        ) if generation == g => {
            retry.messages.push(entry);
            (ResumeState::ConfirmedRetry { generation, attempted_sid, retry, held_error_line, stop_requested }, vec![])
        }

        // Promotion: only when the poisoned sid is the EXACT one this
        // generation attempted — an unrelated/mismatched sid leaves the
        // state untouched (falls through to the catch-all below).
        (
            ResumeState::AwaitingOutcome { generation, attempted_sid, retry, held_error_line, stop_requested },
            ResumeEvent::ResumeUnreachable { generation: g, sid },
        ) if generation == g && attempted_sid == sid => {
            (ResumeState::ConfirmedRetry { generation, attempted_sid, retry, held_error_line, stop_requested }, vec![])
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
            ResumeState::ConfirmedRetry { generation, attempted_sid, retry, held_error_line, stop_requested },
            ResumeEvent::ErrorResultLine { generation: g, line },
        ) if generation == g => {
            let effects = held_error_line.map(|old| vec![ResumeEffect::PersistImmediately(old)]).unwrap_or_default();
            (
                ResumeState::ConfirmedRetry { generation, attempted_sid, retry, held_error_line: Some(line), stop_requested },
                effects,
            )
        }
        (ResumeState::NotTracking { current_generation }, ResumeEvent::ErrorResultLine { line, .. }) => {
            (ResumeState::NotTracking { current_generation }, vec![ResumeEffect::PersistImmediately(line)])
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
            ResumeState::ConfirmedRetry { generation, attempted_sid, retry, held_error_line, .. },
            ResumeEvent::StopRequested { generation: g },
        ) if generation == g => {
            (
                ResumeState::ConfirmedRetry { generation, attempted_sid, retry, held_error_line, stop_requested: true },
                vec![],
            )
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
            (ResumeState::NotTracking { current_generation: generation }, effects)
        }
        (
            ResumeState::ConfirmedRetry { generation, attempted_sid, retry, held_error_line, stop_requested },
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
                //
                // The resume was CONFIRMED unreachable (`ResumeUnreachable`
                // already fired to reach `ConfirmedRetry`) — the retry
                // about to fire clears `session_id` before respawning
                // (`retry_after_resume_failure`), so this is unambiguously
                // a Fresh outcome, known now even though the retry itself
                // hasn't launched yet. See
                // SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md §2.1.
                vec![
                    ResumeEffect::EmitSessionOutcome {
                        outcome: SessionOutcome::Fresh,
                        attempted_sid,
                        actual_sid: None,
                    },
                    ResumeEffect::FireRetry { retry, held_error_line },
                ]
            };
            (ResumeState::NotTracking { current_generation: generation }, effects)
        }
        // reagentx P1 (round 5 on PR #2373): gated on `current_generation
        // == g` — without this, a STALE `ProcessExited` for an
        // already-superseded generation (one a fresh spawn moved past
        // via `SpawnedFresh`/`SpawnedWithResume` before this exact exit
        // event ever arrived) would match unconditionally and incorrectly
        // `PublishDone` (turn_active:false) over a brand-new, actively
        // running generation. A mismatched generation instead falls
        // through to the catch-all below — a safe no-op, same treatment
        // every OTHER state gives a stale event.
        (
            ResumeState::NotTracking { current_generation },
            ResumeEvent::ProcessExited { generation: g },
        ) if current_generation == g => {
            (ResumeState::NotTracking { current_generation }, vec![ResumeEffect::PublishDone])
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

    /// Shorthand for a retry-batch entry with an explicit queue seq
    /// (issue #2365 — retry batches carry identity, not just text).
    fn qentry(seq: u64, json: &str) -> QueuedRetryEntry {
        QueuedRetryEntry { seq, json: json.to_string() }
    }

    fn dummy_retry() -> RetryPayload {
        RetryPayload { config: dummy_config(), messages: vec![qentry(1, "{}")] }
    }

    fn spawned_with_resume(generation: u64) -> ResumeState {
        let (state, effects) = update(
            ResumeState::default(),
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
            ResumeState::default(),
            ResumeEvent::ErrorResultLine { generation: 1, line: "boom".to_string() },
        );
        assert_eq!(state, ResumeState::default());
        assert_eq!(effects, vec![ResumeEffect::PersistImmediately("boom".to_string())]);
    }

    #[test]
    fn fresh_spawn_exit_just_publishes_done() {
        let (state, effects) = update(ResumeState::default(), ResumeEvent::SpawnedFresh { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
        assert!(effects.is_empty());

        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
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
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
        assert_eq!(
            effects,
            vec![ResumeEffect::EmitSessionOutcome {
                outcome: SessionOutcome::Resumed,
                attempted_sid: "dead-sid".to_string(),
                actual_sid: None,
            }]
        );

        // A later, unrelated exit on this now-resolved generation must
        // not retry or dredge up anything.
        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
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
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
        assert_eq!(
            effects,
            vec![
                ResumeEffect::EmitSessionOutcome {
                    outcome: SessionOutcome::Fresh,
                    attempted_sid: "dead-sid".to_string(),
                    actual_sid: None,
                },
                ResumeEffect::FireRetry { retry: dummy_retry(), held_error_line: Some("boom".to_string()) },
            ]
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
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
        assert_eq!(
            effects,
            vec![
                ResumeEffect::EmitSessionOutcome {
                    outcome: SessionOutcome::Fresh,
                    attempted_sid: "dead-sid".to_string(),
                    actual_sid: None,
                },
                ResumeEffect::FireRetry { retry: dummy_retry(), held_error_line: Some("boom".to_string()) },
            ]
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
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
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
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });

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
            update(state, ResumeEvent::MessageAppendedToRetryBatch { generation: 1, entry: qentry(2, "{\"m\":2}") });
        assert!(effects.is_empty());
        match state {
            ResumeState::AwaitingOutcome { retry, .. } => {
                assert_eq!(retry.messages, vec![qentry(1, "{}"), qentry(2, "{\"m\":2}")])
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
            update(state, ResumeEvent::MessageAppendedToRetryBatch { generation: 1, entry: qentry(2, "{\"m\":2}") });
        assert!(effects.is_empty());
        match state {
            ResumeState::ConfirmedRetry { retry, .. } => {
                assert_eq!(retry.messages, vec![qentry(1, "{}"), qentry(2, "{\"m\":2}")])
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
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
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
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
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
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
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
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
        assert_eq!(
            effects,
            vec![
                ResumeEffect::FlushErrorLine("boom".to_string()),
                ResumeEffect::EmitSessionOutcome {
                    outcome: SessionOutcome::Resumed,
                    attempted_sid: "dead-sid".to_string(),
                    actual_sid: None,
                },
            ],
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

        // An unambiguous capture (a genuinely different sid, or a
        // confirmed terminal success) still resolves tracking and
        // flushes the held line even after confirmation.
        let (state, effects) = update(
            state,
            ResumeEvent::SessionCaptured { generation: 1, sid: "dead-sid".to_string(), is_confirmed_success: true },
        );
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
        assert_eq!(
            effects,
            vec![
                ResumeEffect::FlushErrorLine("boom".to_string()),
                ResumeEffect::EmitSessionOutcome {
                    outcome: SessionOutcome::Resumed,
                    attempted_sid: "dead-sid".to_string(),
                    actual_sid: None,
                },
            ]
        );
    }

    // reagentx P1 (round 9 on this PR, also flagged inline by codex): the
    // `ConfirmedRetry` + `SessionCaptured` arm used to resolve
    // UNCONDITIONALLY, discarding the confirmed `retry` payload without
    // ever firing it. `resume_poisoned` (persistent.rs) is a single,
    // non-generation-scoped, PERMANENT field — a lagging stderr-reader
    // task from an OLDER, already-superseded generation can overwrite it
    // to a different sid while THIS generation is still `ConfirmedRetry`,
    // silently disabling `try_capture_session_id`'s poison guard for
    // this generation's own attempted sid and letting a routine same-sid
    // echo (ambiguous, not a confirmed success) reach this arm. That
    // must NOT wipe out the confirmed retry — same unambiguity check as
    // the `AwaitingOutcome` arm.
    #[test]
    fn ambiguous_same_sid_echo_after_confirmation_does_not_discard_the_confirmed_retry() {
        let state = spawned_with_resume(1);
        let (state, _) =
            update(state, ResumeEvent::ResumeUnreachable { generation: 1, sid: "dead-sid".to_string() });
        assert!(matches!(&state, ResumeState::ConfirmedRetry { attempted_sid, .. } if attempted_sid == "dead-sid"));

        // The same attempted sid, echoed on a non-terminal frame — the
        // exact ambiguous case the poison guard normally screens out,
        // reachable here only because a stale, unrelated poison
        // overwrote `resume_poisoned` first.
        let (state, effects) = update(
            state,
            ResumeEvent::SessionCaptured { generation: 1, sid: "dead-sid".to_string(), is_confirmed_success: false },
        );
        assert!(effects.is_empty(), "an ambiguous echo must not flush or fire anything");
        match &state {
            ResumeState::ConfirmedRetry { retry, .. } => {
                assert_eq!(retry.messages, vec![qentry(1, "{}")], "the confirmed retry must survive intact")
            }
            other => panic!("expected the confirmed retry to survive an ambiguous echo, got {other:?}"),
        }

        // The retry still fires normally once the process actually exits.
        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
        assert_eq!(
            effects,
            vec![
                ResumeEffect::EmitSessionOutcome {
                    outcome: SessionOutcome::Fresh,
                    attempted_sid: "dead-sid".to_string(),
                    actual_sid: None,
                },
                ResumeEffect::FireRetry { retry: dummy_retry(), held_error_line: None },
            ]
        );
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
        let (state, effects) = update(ResumeState::default(), ResumeEvent::SpawnedFresh { generation: 1 });
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
        assert!(effects.is_empty());

        let (state, effects) =
            update(state, ResumeEvent::ErrorResultLine { generation: 1, line: "boom".to_string() });
        assert_eq!(effects, vec![ResumeEffect::PersistImmediately("boom".to_string())]);
        let (_, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert_eq!(effects, vec![ResumeEffect::PublishDone]);
    }

    // reagentx P1 (round 5 on this PR): the exact race reagentx flagged —
    // a fresh (or resumed) respawn firing via `SpawnedFresh`/
    // `SpawnedWithResume` while a PRIOR generation is still
    // `AwaitingOutcome`/`ConfirmedRetry` (its own `ProcessExited` hasn't
    // arrived yet — reachable via `persistent.rs`'s
    // `respawn_once_for_leftover_queue` racing the process-waiter task).
    // The prior generation's held error line must reach the user instead
    // of vanishing, and its own eventual (belated) `ProcessExited` must
    // NOT corrupt the new generation's status.
    #[test]
    fn a_fresh_spawn_superseding_an_unresolved_generation_flushes_its_held_error_and_ignores_its_stale_exit() {
        let state = spawned_with_resume(1);
        let (state, _) =
            update(state, ResumeEvent::ErrorResultLine { generation: 1, line: "generation 1's error".to_string() });
        assert!(matches!(&state, ResumeState::AwaitingOutcome { generation: 1, held_error_line: Some(_), .. }));

        // A brand-new spawn (generation 2) supersedes generation 1 before
        // generation 1's own ProcessExited ever arrived.
        let (state, effects) = update(state, ResumeEvent::SpawnedFresh { generation: 2 });
        assert_eq!(
            effects,
            vec![ResumeEffect::FlushErrorLine("generation 1's error".to_string())],
            "generation 1's held error must reach the user, not vanish when generation 2 supersedes it"
        );
        assert_eq!(state, ResumeState::NotTracking { current_generation: 2 });

        // Generation 1's own (belated) ProcessExited must be a safe
        // no-op now — NOT another PublishDone stomping generation 2.
        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert!(
            effects.is_empty(),
            "a stale ProcessExited for a superseded generation must not publish done over the new one"
        );
        assert_eq!(state, ResumeState::NotTracking { current_generation: 2 });

        // Generation 2's OWN eventual exit must still work normally.
        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 2 });
        assert_eq!(state, ResumeState::NotTracking { current_generation: 2 });
        assert_eq!(effects, vec![ResumeEffect::PublishDone]);
    }

    // Same race, but the prior generation had already been CONFIRMED
    // stale (poison_resume won) before the fresh respawn superseded it —
    // its own queued retry must be dropped (not fired), not silently
    // discarded, since a brand-new spawn is already underway via a
    // completely different path.
    #[test]
    fn a_fresh_spawn_superseding_a_confirmed_retry_drops_it_without_double_spawning() {
        let state = spawned_with_resume(1);
        let (state, _) =
            update(state, ResumeEvent::ResumeUnreachable { generation: 1, sid: "dead-sid".to_string() });
        assert!(matches!(state, ResumeState::ConfirmedRetry { generation: 1, .. }));

        let (state, effects) = update(state, ResumeEvent::SpawnedFresh { generation: 2 });
        assert!(
            effects.is_empty(),
            "no held error line existed, so superseding a confirmed (but line-less) retry produces no effect"
        );
        assert_eq!(state, ResumeState::NotTracking { current_generation: 2 });

        let (state, effects) = update(state, ResumeEvent::ProcessExited { generation: 1 });
        assert!(effects.is_empty(), "generation 1's stale exit must not fire its now-superseded retry");
        assert_eq!(state, ResumeState::NotTracking { current_generation: 2 });
    }

    // SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md §2.1: the CLI itself
    // can silently roll to a DIFFERENT session id (rather than failing
    // outright) before `ResumeUnreachable` ever gets a chance to fire — the
    // `sid != attempted_sid` branch of the unambiguity check. This must
    // still be reported as Fresh, with `actual_sid` carrying the id the CLI
    // actually landed on.
    #[test]
    fn session_captured_with_a_different_sid_emits_fresh_outcome_with_actual_sid() {
        let state = spawned_with_resume(1);
        let (state, effects) = update(
            state,
            ResumeEvent::SessionCaptured {
                generation: 1,
                sid: "brand-new-sid".to_string(),
                is_confirmed_success: false,
            },
        );
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
        assert_eq!(
            effects,
            vec![ResumeEffect::EmitSessionOutcome {
                outcome: SessionOutcome::Fresh,
                attempted_sid: "dead-sid".to_string(),
                actual_sid: Some("brand-new-sid".to_string()),
            }]
        );
    }

    // Regression guard for SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md
    // §2.1's "deliberately not touched" call: a fresh spawn with no resume
    // attempt at all (no session existed yet — nothing to lose), and a
    // resume attempt that's never confirmed either way (auth failure etc.,
    // §2.1's `unrelated_error_never_confirmed_flushes_instead_of_retrying`
    // case), must NOT emit a session-outcome event — there's no positively-
    // known outcome to report in either case.
    #[test]
    fn spawned_fresh_and_never_confirmed_paths_never_emit_a_session_outcome() {
        let (state, effects) = update(ResumeState::default(), ResumeEvent::SpawnedFresh { generation: 1 });
        assert!(!effects.iter().any(|e| matches!(e, ResumeEffect::EmitSessionOutcome { .. })));

        let state2 = spawned_with_resume(2);
        let (state2, _) =
            update(state2, ResumeEvent::ErrorResultLine { generation: 2, line: "auth failed".to_string() });
        let (_, effects) = update(state2, ResumeEvent::ProcessExited { generation: 2 });
        assert!(
            !effects.iter().any(|e| matches!(e, ResumeEffect::EmitSessionOutcome { .. })),
            "a never-confirmed resume attempt has no positively-known outcome to report"
        );

        // Keep `state` (generation 1) used so the compiler doesn't flag it —
        // its only purpose above is exercising the SpawnedFresh path.
        assert_eq!(state, ResumeState::NotTracking { current_generation: 1 });
    }
}
