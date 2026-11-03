// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AgentPaneCommand, AgentPaneEvent, AgentPaneState, INTERRUPT_TIMEOUT_MS, KindBeforeDisconnect, LIVENESS_RECOVERY_MS, NO_RATE_LIMIT_WAIT, ReducerResult, STUCK_THRESHOLD_MS, SUBMIT_TIMEOUT_MS, TurnPhase, type DisconnectReason } from "./types";
import { settledActivity, bumpEvent } from "./reducer-helpers";

// ── Init lifecycle (gap 1) ─────────────────────────────────
export function onInitStart(state: AgentPaneState): ReducerResult {
    // Idempotent: only emits when first entering pending. Already
    // in pending → no-op (same ref). Already terminal → no-op
    // (we don't reset back from InitReady/InitFailed).
    return { state, events: [] };
}

/** `InitReady`. */
export function onInitReady(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "InitReady" }>): ReducerResult {
    if (state.initPhase.kind !== "InitPending") {
        // Already terminal — no-op. Out-of-order InitReady after
        // an earlier InitFailed (or a duplicate InitReady) is
        // silently dropped to keep transitions one-way.
        return { state, events: [] };
    }
    return {
        state: { ...state, initPhase: { kind: "InitReady", at: command.at, ...(command.historyDamaged && { historyDamaged: true as const }) } },
        events: [{ type: "init-ready" }],
    };
}

/** `InitFailed`. */
export function onInitFailed(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "InitFailed" }>): ReducerResult {
    if (state.initPhase.kind !== "InitPending") {
        // Same one-way rule as InitReady — once terminal, stay.
        return { state, events: [] };
    }
    return {
        state: {
            ...state,
            initPhase: { kind: "InitFailed", at: command.at, reason: command.reason },
        },
        events: [{ type: "init-failed", reason: command.reason }],
    };
}

// ── Stream lifecycle ───────────────────────────────────────
export function onStreamSubscribe(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "StreamSubscribe" }>): ReducerResult {
    // Dual-write: if a turn was Submitting, the subscription is
    // its expected hand-off to Streaming. If already Streaming,
    // the resubscribe IS fresh activity — refresh
    // `lastEventMs` to `command.at` so the stuck-stream
    // diagnostic (`StreamWatchdogTick` → `stream-stuck`) measures
    // idle time from the resubscribe, not from a stale prior
    // event. (P1 on #995 from reagent + codex.)
    //
    // PR F: a subscribe from `Disconnected` clears the
    // disconnect — we land in `Idle` (not back in the working
    // set; the lost turn is gone, the next `TurnStart` promotes
    // to Submitting). Done stays Done (terminal). Idle stays
    // Idle (no-op).
    const nextPhase: TurnPhase =
        state.turnPhase.kind === "Submitting"
            ? {
                  kind: "Streaming",
                  bufferSize: state.streaming.bufferSize,
                  toolsActive: 0,
                  lastEventMs: command.at,
              }
            : state.turnPhase.kind === "Streaming"
                ? {
                      ...state.turnPhase,
                      lastEventMs: command.at,
                  }
                : state.turnPhase.kind === "Disconnected"
                    ? { kind: "Idle" }
                    : state.turnPhase;
    const events: AgentPaneEvent[] = [
        { type: "stream-subscribed", at: command.at },
    ];
    return {
        state: {
            ...state,
            streaming: {
                ...state.streaming,
                lastEventTime: command.at,
            },
            lastEventMs: command.at,
            turnPhase: nextPhase,
        },
        events,
    };
}

/** `StreamUnsubscribe`. */
export function onStreamUnsubscribe(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "StreamUnsubscribe" }>): ReducerResult {
    // If we were working (Submitting/Streaming/Interrupting),
    // surface a Disconnected phase so the view can show
    // re-attach UX. Otherwise (Idle / Done / already
    // Disconnected) the unsubscribe is idempotent — return the
    // same state ref so reactive consumers don't see a phantom
    // tick. Spec §6.4 / PR F.
    const k = state.turnPhase.kind;
    const wasWorking =
        k === "Submitting" || k === "Streaming" || k === "Interrupting";
    if (!wasWorking) {
        // reagent P1 on PR #2928: `pendingCompactionPing` CAN be set
        // here — that's exactly the buffering condition
        // CompactionStarted adds for Idle/Disconnected/Done.completed
        // (see that case). The old "no in-flight turn so per-turn
        // sidecars are already cleared" premise predates this field
        // and no longer holds for it: an unsubscribe while a ping is
        // buffered (e.g. tab backgrounded pre-reconciliation) must
        // still discard it, or a later resubscribe + resumed content
        // (StreamFlushObserved's Idle→Streaming promotion) would
        // retroactively — and wrongly — promote a stale ping onto
        // whatever unrelated turn comes next.
        if (state.pendingCompactionPing == null) {
            // True same-ref no-op for Idle / Done / Disconnected
            // with nothing buffered — the unsubscribe is non-news.
            return { state, events: [] };
        }
        return {
            state: { ...state, pendingCompactionPing: null },
            events: [{ type: "stream-unsubscribed", at: command.at }],
        };
    }
    const reason: DisconnectReason = "stream-unsubscribed";
    const lastKind = k as KindBeforeDisconnect;
    const nextPhase: TurnPhase = {
        kind: "Disconnected",
        lastKind,
        lastConnectedAt: command.at,
        reason,
    };
    return {
        state: {
            ...state,
            // Per-turn sidecar cleanup mirrors the bounded
            // force-transition arms (InterruptTimeoutElapsed /
            // SubmitTimeoutElapsed): a forced
            // exit from a working state settles the animation
            // and clears tool / token chips so the header
            // doesn't show ghosts.
            currentTool: null,
            currentToolArg: null,
            turnTokens: null,
            lastEventMs: null,
            turnPhase: nextPhase,
            // reagent P1 on PR #2378: a disconnect mid-compaction
            // (crash, network drop, reconnect race) means the
            // matching CompactionBoundary will never arrive to
            // clear this — without this, the composer strip is
            // stuck showing "Compacting… Ns" forever, surviving
            // the reconnect and every subsequent turn.
            compacting: null,
            pendingCompactionPing: null,
        },
        events: [
            { type: "stream-unsubscribed", at: command.at },
            {
                type: "stream-disconnected",
                at: command.at,
                lastKind,
                reason,
            },
        ],
    };
}

/** `StreamFlushObserved`. */
export function onStreamFlushObserved(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "StreamFlushObserved" }>): ReducerResult {
    // Gate: only mutate if the stream is currently subscribed.
    // PR G — previously `state.streaming.active`; the
    // `lastEventMs !== null` check is exactly equivalent
    // (both moved in lockstep on subscribe/unsubscribe).
    if (state.lastEventMs == null) {
        return { state, events: [] };
    }
    // FIX 1 — shell OUTPUT alone is not evidence this turn is alive.
    // The flush queue batches six independent queues and dispatches one
    // StreamFlushObserved if ANY of them had content. A live background
    // shell writing to its dock row therefore refreshed lastEventMs on
    // every flush, holding idleSinceMs permanently below the recovery
    // threshold — the pane sat in Streaming for as long as the process
    // lived (5h observed, and the same shape as the 12h incident in
    // retro-persistent-agent-working-status-stuck-2026-07-16.md).
    //
    // Nothing about that output implies the MODEL is working, so it
    // neither bumps the idle clock nor promotes the phase. The event is
    // still emitted: other consumers legitimately track flushes.
    if (command.turnRelevant === false) {
        return {
            state,
            events: [{ type: "stream-flush-observed", addedCount: command.addedCount }],
        };
    }
    const newBuf = state.streaming.bufferSize + command.addedCount;
    // Dual-write: while Streaming, mirror bufferSize + lastEventMs.
    // PROMOTE to Streaming from Submitting (the normal hand-off) AND
    // from Idle / Disconnected. `StreamFlushObserved` is only dispatched
    // for LIVE stream content (never history replay), so a flush is
    // proof the agent is producing output — it must re-enter the working
    // set. After a stream drop + resubscribe (e.g. an agent kill+respawn
    // during a long upstream stall) the phase lands in Idle/Disconnected,
    // and without this promotion the "in progress" indicator stays OFF
    // while output streams in. Done.completed is included: session_end
    // fires after every model API round, so Done.completed can mean
    // "first round of a multi-round tool-continuation finished" — live
    // flush content arriving in that state is proof the agent picked up
    // another round. Done.errored / Done.stopped / Done.interrupted are
    // excluded: a submit-timeout or user-stop followed by a late stray
    // flush must not re-activate the busy indicator on a failed turn.
    // Interrupting (user is stopping) is also intentionally excluded.
    const nextPhase: TurnPhase =
        state.turnPhase.kind === "Streaming"
            ? {
                  ...state.turnPhase,
                  bufferSize: newBuf,
                  lastEventMs: command.at,
                  // Clear a stale rate-limit wait on ANY observable
                  // stream activity, not just tool/token events —
                  // `bumpEvent` already does this; this branch
                  // previously spread the whole phase forward
                  // unchanged, so a rate-limited turn whose very
                  // next activity was plain streamed text (no
                  // intervening tool call) kept showing "Rate
                  // limited — retrying…" long after the agent had
                  // resumed normal streaming (reported false
                  // positive, see
                  // ANALYSIS_AGENT_INPUT_LIFECYCLE_RATELIMIT_SENDNOW_2026_07_06.md).
                  ...NO_RATE_LIMIT_WAIT,
              }
            : state.turnPhase.kind === "Submitting"
                || state.turnPhase.kind === "Idle"
                || state.turnPhase.kind === "Disconnected"
                || (state.turnPhase.kind === "Done"
                    && state.turnPhase.outcome === "completed"
                    // FIX 2 — resurrect a COMPLETED turn only for a
                    // flush that actually added document nodes. The
                    // multi-round continuation this branch exists for
                    // (#2420) always opens with a new node; a flush
                    // carrying only chunk appends or shell traffic is
                    // not a new round, and treating it as one is how a
                    // finished turn silently went back to "Working…".
                    && command.addedCount > 0)
                ? {
                      kind: "Streaming",
                      bufferSize: newBuf,
                      toolsActive: 0,
                      lastEventMs: command.at,
                  }
                : state.turnPhase;
    const events: AgentPaneEvent[] = [
        { type: "stream-flush-observed", addedCount: command.addedCount },
    ];
    // SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md: the
    // other authoritative promotion a `pendingCompactionPing` can be
    // retroactively applied against — resumed live content is proof
    // this same turn is genuinely active, the same standard the
    // promotion ternary above already uses.
    const promoted = nextPhase.kind === "Streaming" && state.turnPhase.kind !== "Streaming";
    const pending = promoted ? state.pendingCompactionPing : null;
    if (pending != null) {
        events.push({ type: "compaction-started", trigger: pending.trigger });
    }
    return {
        state: {
            ...state,
            streaming: {
                ...state.streaming,
                bufferSize: newBuf,
                lastEventTime: command.at,
            },
            lastEventMs: command.at,
            turnPhase: nextPhase,
            compacting:
                pending != null ? { trigger: pending.trigger, startedAt: pending.startedAt } : state.compacting,
            pendingCompactionPing: promoted ? null : state.pendingCompactionPing,
        },
        events,
    };
}

/** `StreamWatchdogTick`. */
export function onStreamWatchdogTick(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "StreamWatchdogTick" }>, nowMs: number): ReducerResult {
    // No-op when stream unsubscribed (lastEventMs null also
    // covers "no event seen yet" — same field since PR G
    // dropped the redundant `streaming.active` boolean).
    if (state.lastEventMs == null) {
        return { state, events: [] };
    }
    // Codex P1 on PR #2378 (round 2): CompactionStarted only bumps
    // lastEventMs ONCE, at the moment compaction begins — it isn't
    // re-bumped on every tick while compaction is still running.
    // The captured real example (spec doc §2) took ~232s: past
    // STUCK_THRESHOLD_MS (45s) alone that's a cosmetic false
    // "stream-stuck" warning, but past LIVENESS_RECOVERY_MS (180s)
    // it would force-demote a perfectly healthy, actively-
    // compacting Streaming turn to Idle out from under it. Suspend
    // the watchdog entirely for as long as `compacting` is set —
    // CompactionBoundary (or any of the other lifecycle events that
    // clear `compacting`, see the tests above) re-arms it.
    if (state.compacting != null) {
        return { state, events: [] };
    }
    const idleSinceMs = command.nowMs - state.lastEventMs;
    if (idleSinceMs < STUCK_THRESHOLD_MS) {
        return { state, events: [] };
    }
    // Liveness recovery (SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29).
    // A `Streaming` turn that goes idle too long with no tool running is
    // hung: its terminal `session_end` was never observed. For persistent
    // agents no `ControllerStatus: done` will ever arrive to clear it (the
    // process doesn't exit between turns), so the watchdog itself
    // transitions out of "Working". Land in `Idle` (turn abandoned) — NOT
    // a synthetic `Done.completed`, which would feed false stats to session
    // digests. If real activity arrives after recovery, `bumpEvent`
    // re-promotes Idle → Streaming, so a late recovery self-corrects.
    //
    // Threshold depends on the wait state:
    //  - normal stall → LIVENESS_RECOVERY_MS (a running tool keeps
    //    `lastEventMs` fresh, so an active tool never reaches it).
    //  - rate-limit stall → retryAfterMs + LIVENESS_RECOVERY_MS. A genuine
    //    429 backoff re-emits `provider_waiting` within `retryAfterMs`
    //    (refreshing `lastEventMs`), so it only accumulates this much idle
    //    if the retry loop itself stalled — the Claude CLI emitted one
    //    `rate_limit_event` then went silent with no follow-up event,
    //    `session_end`, or process exit. Waiting out the advertised retry
    //    window PLUS the liveness window guarantees a long-but-live backoff
    //    is never mistaken for a hang. (Pre-fix, rate-limited turns were
    //    excluded from recovery entirely — see
    //    docs/retro/retro-busy-animation-stuck-on-429-2026-06-24.md.)
    // Interrupting/Submitting are left to their own bounded timers
    // (INTERRUPT_TIMEOUT_MS / SUBMIT_TIMEOUT_MS).
    const phase = state.turnPhase;
    if (phase.kind === "Streaming" && phase.toolsActive === 0) {
        const recoverThresholdMs =
            phase.waitingReason === "rate_limited"
                ? (phase.retryAfterMs ?? 0) + LIVENESS_RECOVERY_MS
                : LIVENESS_RECOVERY_MS;
        if (idleSinceMs >= recoverThresholdMs) {
            return {
                state: {
                    ...state,
                    currentTool: null,
                    currentToolArg: null,
                    // turnTokens stay: a late session_end still merges
                    // them (see ReconcileTurnActive below).
                    turnPhase: { kind: "Idle" },
                },
                events: [
                    {
                        type: "working-recovered",
                        idleSinceMs,
                        thresholdMs: recoverThresholdMs,
                    },
                ],
            };
        }
    }
    return {
        state,
        events: [
            {
                type: "stream-stuck",
                idleSinceMs,
                thresholdMs: STUCK_THRESHOLD_MS,
                // FIX 3 — the bar this warning is measured against
                // (45s) is NOT the bar for recovery (180s, or more
                // under a rate-limit backoff). Reporting only the
                // former reads as "past the threshold and still not
                // recovering", which is exactly how this stall was
                // misdiagnosed while it was happening.
                recoverThresholdMs:
                    state.turnPhase.kind === "Streaming"
                    && state.turnPhase.waitingReason === "rate_limited"
                        ? (state.turnPhase.retryAfterMs ?? 0) + LIVENESS_RECOVERY_MS
                        : LIVENESS_RECOVERY_MS,
            },
        ],
    };
}

/** `ReconcileTurnActive`. */
export function onReconcileTurnActive(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ReconcileTurnActive" }>): ReducerResult {
    if (command.active) {
        // Promote from the mount-default Idle (this seed's original
        // purpose — covers the window before the live stream has
        // necessarily subscribed yet, see the type's doc comment in
        // types.ts) OR from a settled Done.completed episode. The
        // Done.completed case matters for the focus-triggered
        // reconcile (agent-view.tsx): a pane can be showing "Worked"
        // while backgrounded, a genuinely new turn starts, AND the
        // live turn-start signal is also missed — without this, the
        // authoritative "yes, a turn really is active" RPC response
        // this command carries would silently no-op, leaving the
        // pane stuck on "Worked" despite real work in progress
        // (reagent P1 on the PR that added the focus reconcile).
        // Applies the SAME standard StreamFlushObserved already uses
        // for live stream content proving a continuation — this
        // authoritative snapshot is an equally strong (arguably
        // stronger) signal of truth. Other Done outcomes (stopped/
        // errored/interrupted) stay excluded, matching
        // StreamFlushObserved: a failed/stopped turn must not be
        // silently re-activated. Never overrides Streaming/
        // Submitting/Interrupting/Disconnected — those already
        // reflect a real stream/user event this snapshot must not
        // clobber. Deliberately does NOT gate on `state.lastEventMs`
        // the way TurnStart / StreamFlushObserved do, for the same
        // reason the original Idle case didn't.
        const canPromote =
            state.turnPhase.kind === "Idle" ||
            (state.turnPhase.kind === "Done" && state.turnPhase.outcome === "completed");
        if (!canPromote) {
            return { state, events: [] };
        }
        // SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md:
        // this is one of the two authoritative promotions that a
        // `pendingCompactionPing` (buffered by `CompactionStarted`
        // above, because it arrived before THIS confirmation)
        // retroactively applies against — this RPC response IS the
        // signal the ping was waiting for.
        const pending = state.pendingCompactionPing;
        const events: AgentPaneEvent[] = [{ type: "turn-active-reconciled" }];
        if (pending != null) {
            events.push({ type: "compaction-started", trigger: pending.trigger });
        }
        return {
            state: {
                ...state,
                // turnTokens stay, as on TurnStart: the next turn can be
                // marked active before the last one's session_end has
                // come through the stream.
                turnPhase: {
                    kind: "Streaming",
                    bufferSize: 0,
                    toolsActive: 0,
                    lastEventMs: command.at,
                },
                compacting:
                    pending != null
                        ? { trigger: pending.trigger, startedAt: pending.startedAt }
                        : state.compacting,
                pendingCompactionPing: null,
            },
            events,
        };
    }
    // active === false: the backend's authoritative turn_active (flipped
    // false only on the CLI's `result` event = genuine turn-end) says
    // the turn is over. Demote a STUCK Streaming phase to Idle. The
    // frontend normally reaches Done itself via session_end; this only
    // matters when it missed that event and would stay Streaming forever
    // (Agent1 stuck-"Working" / Agent2 stuck-"Queued" — the latter's
    // held-message flush is gated on the phase reaching Idle/Done, so a
    // stuck Streaming strands the queue). ONLY Streaming: Submitting is
    // left to SUBMIT_TIMEOUT (and is where the send-race lives — a local
    // TurnStart racing a backend that hasn't marked the turn active yet,
    // which would arrive here as a stale active:false), Interrupting to
    // INTERRUPT_TIMEOUT, and Done/Idle/Disconnected are already correct.
    // Clears currentTool like the liveness watchdog's recovery, but on
    // an authoritative signal rather than a timeout. Both keep
    // turnTokens for the turn's own TurnEnd (see below).
    //
    // `pendingCompactionPing` is cleared REGARDLESS of whether
    // turnPhase itself changes below (reagent P1 + codex P2 on PR
    // #2928, against the two prior hook-only fix attempts): this
    // authoritative "no turn is active" confirmation proves any
    // buffered ping was for a turn that's genuinely not running,
    // whether or not the pane's phase happens to already be Idle
    // (the common case — a same-ref no-op for `turnPhase`, but this
    // command is still processed as a discrete event, unlike a
    // hook that can only observe the resulting value).
    if (state.turnPhase.kind !== "Streaming") {
        if (state.pendingCompactionPing == null) {
            return { state, events: [] };
        }
        return {
            state: { ...state, pendingCompactionPing: null },
            events: [{ type: "turn-inactive-reconciled", at: command.at }],
        };
    }
    return {
        state: {
            ...state,
            currentTool: null,
            currentToolArg: null,
            activity: settledActivity(state.activity),
            // turnTokens stay. srv sends this push as soon as it reads
            // the CLI's `result` line, before it forwards that line, so
            // the stream is usually still delivering this turn: its
            // session_end, and sometimes calls before it. Clearing them
            // here left TurnEnd with no live tokens, and the footer
            // showed every call's input summed. Only the stream ends
            // them: TurnEnd, or a new CLI session (StreamSessionStarted).
            turnPhase: { kind: "Idle" },
            // Same reasoning as the other authoritative terminal
            // transitions above: the backend has just confirmed
            // there is no active turn, so whatever compaction the
            // frontend still thought was in flight is stale.
            compacting: null,
            pendingCompactionPing: null,
        },
        events: [{ type: "turn-inactive-reconciled", at: command.at }],
    };
}
