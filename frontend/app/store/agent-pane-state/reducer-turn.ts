// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AgentPaneCommand, AgentPaneEvent, AgentPaneState, ReducerResult, SUBMIT_TIMEOUT_MS, TurnOutcome, workingFromPhase, IDLE_ACTIVITY } from "./types";
import { isNewerLedger } from "./turn-ledger";
import { carryPass, settledActivity, mergeStats, accumulateStats } from "./reducer-helpers";

/** `TurnStart`. */
export function onTurnStart(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "TurnStart" }>): ReducerResult {
    // Invariant 1: can't start a turn without a subscribed
    // stream. PR G: `state.lastEventMs !== null` replaces the
    // dropped `state.streaming.active` boolean — both moved
    // in lockstep so the gate is unchanged.
    if (state.lastEventMs == null) {
        return {
            state,
            events: [
                {
                    type: "turn-start-suppressed",
                    reason: "stream not active",
                },
            ],
        };
    }
    // Invariant 2: can't start a turn while still loading history.
    // After init failure we fail open — the live stream may still
    // work even if history fetch hung, and blocking the user
    // would be worse than silently dropping a stale send.
    if (state.initPhase.kind === "InitPending") {
        return {
            state,
            events: [
                {
                    type: "turn-start-suppressed",
                    reason: "init still loading",
                },
            ],
        };
    }
    // PR D: only schedule the submit watchdog when we actually
    // CROSS INTO Submitting (not when we're already there — that
    // would double-arm the timeout). Mirrors the PR C entry
    // pattern for the interrupt watchdog. In practice TurnStart
    // is dispatched once per user-send so we expect this branch
    // ~always, but defence in depth keeps the contract clean for
    // any future re-entrant caller.
    const enteringSubmitting = state.turnPhase.kind !== "Submitting";
    const events: AgentPaneEvent[] = [
        { type: "turn-started", at: command.at },
    ];
    if (enteringSubmitting) {
        events.push({
            type: "schedule-submit-timeout",
            deadlineMs: command.at + SUBMIT_TIMEOUT_MS,
        });
    }
    // A fresh turn starting always ends any prior failure episode —
    // this is what the failure-recovery hook used to reconstruct by
    // watching raw ControllerStatus transitions (`awaitingVerdict`);
    // TurnStart already knows "a new turn is beginning" directly, so
    // there's no need to re-derive it from the outside. Emit
    // failure-cleared only when there was actually something to
    // clear, matching the other no-op-vs-event conventions in this
    // reducer (e.g. DetailsExpand/DetailsCollapse below).
    if (state.failure) {
        events.push({ type: "failure-cleared" });
    }
    return {
        state: {
            ...state,
            sessionStats: null, // clear stale stats from prior turn
            // turnTokens stay: a queued message drains as soon as the
            // turn-ended push makes the pane Idle, often before that
            // turn's session_end has come through the stream.
            lastEventMs: command.at,
            failure: null,
            // Submitting until the first stream event /
            // subscribe transitions us to Streaming.
            turnPhase: {
                kind: "Submitting",
                submittedAt: command.at,
                pendingContent: command.content ?? "",
                hidden: command.hidden,
            },
            // See pendingCompactTurn's doc comment (types.ts) — always
            // set explicitly (never left stale) so CompactionBoundary
            // can tell whether THIS pane's currently-working turn is
            // the synthetic one the "Compact now" button opened.
            pendingCompactTurn: (command.content ?? "").trim() === "/compact",
            // Previously auto-collapsed the details panel on send (don't
            // obscure the turn start). The panel now also hosts a live
            // interactive shell (AgentShellSubblock) whose whole point is
            // surviving across turns, so force-closing it on every message
            // would fight that — leave detailsOpen as the user set it.
            //
            // SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md
            // (reagent P1 + codex P2 on PR #2928): a fresh, genuinely
            // NEW user-initiated turn is NOT the turn any buffered
            // ping was about — discard rather than let it survive to
            // be wrongly consumed by StreamFlushObserved's own
            // Submitting→Streaming promotion moments later (which
            // fires for practically every TurnStart), which would
            // falsely set `compacting` on this unrelated turn.
            pendingCompactionPing: null,
        },
        events,
    };
}

/** `TurnEnd`. */
export function onTurnEnd(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "TurnEnd" }>, nowMs: number): ReducerResult {
    // PR G: replaces the legacy `state.stopping` boolean. A
    // stop-in-flight is exactly `turnPhase.kind === "Interrupting"`
    // since RequestStop is the only command that enters
    // Interrupting and the same arms that previously cleared
    // `stopping` are the ones that leave Interrupting.
    const stoppingWasSet = state.turnPhase.kind === "Interrupting";
    // Merge stats with live turn-tokens (mirrors prior finalizeTurn
    // logic — see PR #549 reagent/codex P1 reference).
    const merged = mergeStats(command.stats, state.turnTokens);
    // Codex P1 on #991: when an `InterruptTimeoutElapsed` has
    // already forced us to `Done.interrupted` (clearing `stopping`
    // in the process), a late backend `TurnEnd` ack would see
    // `stoppingWasSet === false` and classify the outcome as
    // "completed", overwriting the forced interrupted state.
    // First-done-wins: if we're already in `Done`, preserve the
    // existing outcome and only refresh the stats sidecar. The
    // late ack is confirmatory, not authoritative.
    //
    // PR F extends the guard to `Disconnected` as well — Option A
    // from the PR F spec: a late TurnEnd that arrives after the
    // stream has already torn down (e.g. backend pushed a final
    // session_end into a buffer right before the PTY died, then
    // the buffered ack drains) must NOT overwrite the disconnect
    // with a synthetic Done. The disconnect is the authoritative
    // outcome; the late ack is informational. Same-ref no-op
    // preserves both the phase and the cleared legacy fields.
    if (state.turnPhase.kind === "Disconnected") {
        // reagent P1 on PR #2928: same gap as StreamUnsubscribe's
        // early-return branch above — `pendingCompactionPing` can be
        // set while Disconnected (a promotable-later phase per
        // CompactionStarted's buffering condition), and this late
        // TurnEnd ack IS an authoritative "this turn genuinely
        // ended" signal even though the phase itself stays
        // Disconnected. Clear it so a later StreamSubscribe +
        // resumed content can't retroactively promote a stale ping
        // onto whatever unrelated turn comes next.
        if (state.pendingCompactionPing == null) {
            return { state, events: [] };
        }
        return { state: { ...state, pendingCompactionPing: null }, events: [] };
    }
    // Dual-write outcome: stop-in-flight → "stopped"; otherwise
    // "completed". "interrupted" / "errored" are reserved for
    // bounded force-transition arms (InterruptTimeoutElapsed,
    // SubmitTimeoutElapsed).
    //
    // TS narrowing: read the phase via a local so the
    // `.kind === "Done"` ternary below narrows `outcome` /
    // `finishedAt` accesses (a `const alreadyDone = …` boolean
    // wouldn't carry the narrow into the ternary branches).
    const phase = state.turnPhase;
    const outcome: TurnOutcome = phase.kind === "Done"
        ? phase.outcome
        : stoppingWasSet
            ? "stopped"
            : "completed";
    const finishedAt = phase.kind === "Done"
        ? phase.finishedAt
        : nowMs;
    return {
        state: {
            ...state,
            sessionStats: merged,
            sessionTotals: accumulateStats(state.sessionTotals, merged),
            turnCarry: carryPass(state.turnCarry, state.turnLedger?.turnId, merged),
            currentTool: null,
            currentToolArg: null,
            turnTokens: null,
            activity: settledActivity(state.activity),
            turnPhase: {
                kind: "Done",
                outcome,
                finishedAt,
            },
            // reagent P1 on PR #2378: the turn ending (however it
            // ended) means whatever compaction was in flight is
            // moot — a CompactionBoundary for it, if it ever
            // arrives, would be stale. See the same note on
            // StreamUnsubscribe above.
            compacting: null,
            pendingCompactionPing: null,
        },
        events: [
            {
                type: "turn-ended",
                outcome,
                statsMerged: merged != null,
                stoppingCleared: stoppingWasSet,
            },
        ],
    };
}

// In stream order, a session's `init` comes after the last turn's
// `result` and before the next turn's calls, so clearing here never
// drops tokens a coming TurnEnd needs.
export function onStreamSessionStarted(state: AgentPaneState): ReducerResult {
    if (state.turnTokens == null) return { state, events: [] };
    return { state: { ...state, turnTokens: null }, events: [] };
}

/** `TurnReset`. */
export function onTurnReset(state: AgentPaneState): ReducerResult {
    return {
        state: {
            ...state,
            sessionStats: null,
            sessionTotals: null,
            currentTool: null,
            currentToolArg: null,
            turnTokens: null,
            activity: IDLE_ACTIVITY,
            context: null,
            contextSeedable: false,
            // TurnReset is a wholesale clear → Idle. The
            // working/stopping cascade lives entirely on
            // turnPhase since PR G.
            turnPhase: { kind: "Idle" },
            // reagent P1 on PR #2378: same "wholesale clear"
            // reasoning extends to `compacting` — a reset must
            // not leave a stale "Compacting…" readout behind.
            compacting: null,
            pendingCompactionPing: null,
        },
        events: [{ type: "turn-reset" }],
    };
}

/** `TurnObserved`. */
export function onTurnObserved(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "TurnObserved" }>): ReducerResult {
    if (!isNewerLedger(command.ledger, state.turnLedger)) return { state, events: [] };
    return { state: { ...state, turnLedger: command.ledger }, events: [] };
}

/** `TurnStartFailed`. */
export function onTurnStartFailed(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "TurnStartFailed" }>): ReducerResult {
    // Deliberately touches ONLY turnPhase (+ compacting, see below)
    // — see this command's doc
    // comment in types.ts for why TurnReset's wholesale clear is
    // wrong here (a transient send failure, or a slash/bang command
    // handled locally, must not wipe sessionStats/sessionTotals/
    // context accumulated by prior real turns in this same pane).
    // `compacting` IS cleared, unlike the fields above: codex P2
    // on PR #2378 (round 8) — a `compaction_started` ping can
    // land while this (new, doomed) turn attempt is briefly
    // Submitting (round 5's workingFromPhase gate explicitly
    // permits Submitting), then this arm reverts to Idle without
    // ever getting a matching CompactionBoundary for a turn that
    // never really started — same bug class as
    // SubmitTimeoutElapsed/InterruptTimeoutElapsed/
    // ReconcileTurnActive above, missed here in round 4 on the
    // (wrong) assumption that a synchronously-failed turn-start
    // could never race a live MPS ping.
    return {
        state: { ...state, turnPhase: { kind: "Idle" }, compacting: null, pendingCompactionPing: null },
        events: [{ type: "turn-start-failed" }],
    };
}
