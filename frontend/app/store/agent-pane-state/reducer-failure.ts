// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AgentPaneCommand, AgentPaneState, ReducerResult, TurnOutcome, TurnPhase, workingFromPhase } from "./types";
import { failureDropped } from "./failure-precedence";
import { readingPostdates } from "./reducer-helpers";

/** `FailureObserved`. */
export function onFailureObserved(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "FailureObserved" }>): ReducerResult {
    if (failureDropped(state, command)) return { state, events: [] }; // failure-precedence.ts
    // Unconditionally end a working turn: a backend failure
    // classification is authoritative regardless of whether the
    // underlying CLI process ever exits (persistent-mode agents
    // don't, between turns — see useAgentStream.ts's process-exit
    // grace timer, which only covers the crash-exit case). Without
    // this, a rate-limited (or otherwise failed) persistent-mode
    // turn left `turnPhase` stuck in `Streaming` until the ~3-minute
    // liveness-recovery watchdog eventually cleared it — the
    // "stuck Waiting after a rate-limit interruption" bug.
    const turnWasEnded = workingFromPhase(state.turnPhase);
    const nextPhase: TurnPhase = turnWasEnded
        ? { kind: "Done", outcome: "errored", finishedAt: command.at }
        : state.turnPhase; // already idle — a stray/late event; leave phase alone
    return {
        state: {
            ...state,
            turnPhase: nextPhase,
            currentTool: turnWasEnded ? null : state.currentTool,
            currentToolArg: turnWasEnded ? null : state.currentToolArg,
            turnTokens: turnWasEnded ? null : state.turnTokens,
            failure: {
                data: command.failure,
                at: command.at,
                // Defaults to true when the command omits it — every
                // backend-classified failure is by construction the
                // outcome of a turn that ran. Only the pane's own
                // synthetic pre-launch auth failure passes false.
                // See PaneFailure.turnAttempted (types.ts).
                turnAttempted: command.turnAttempted ?? true,
                ...(command.source ? { source: command.source } : {}),
            },
            // A failure ends the turn like TurnEnd/TurnReset/RequestStop/StreamUnsubscribe,
            // so the compaction in flight is moot too (else a mid-compaction error leaves
            // "Compacting…" stuck, #2378).
            compacting: turnWasEnded ? null : state.compacting,
            pendingCompactionPing: turnWasEnded ? null : state.pendingCompactionPing,
        },
        events: [
            { type: "failure-observed", code: command.failure.code, turnWasEnded },
        ],
    };
}

/** `FailureCleared`. */
export function onFailureCleared(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "FailureCleared" }>): ReducerResult {
    if (!state.failure) return { state, events: [] };
    if (command.onlyCode && state.failure.data.code !== command.onlyCode) return { state, events: [] };
    return {
        state: { ...state, failure: null },
        events: [{ type: "failure-cleared", ...(command.reason ? { reason: command.reason } : {}) }],
    };
}

/** `CompactionStarted`. */
export function onCompactionStarted(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "CompactionStarted" }>): ReducerResult {
    // Mirrors ProviderWaiting's "observable activity" bump — a
    // long compaction with no other stream output must not trip
    // the stuck-stream watchdog. No-op if the stream isn't
    // subscribed (a stray/late event after teardown).
    //
    // reagent P1 on PR #2378 (round 5): also no-op unless the turn
    // is actually in the working set (Submitting/Streaming/
    // Interrupting). `compaction_started` arrives over a SEPARATE
    // transport (MPS: HTTP publish -> broker -> websocket) from the
    // primary NDJSON stream carrying TurnEnd/compact_boundary, so
    // it can race and land AFTER that same turn's TurnEnd already
    // fired. Every "clear compacting" fix added across rounds 1-4
    // is itself gated on transitioning OUT of a working phase — if
    // `compacting` gets set while the pane is already Idle/Done,
    // none of those transitions ever fire again to clear it, and it
    // stays orphaned until the pane's next full TurnEnd/TurnReset —
    // the exact bug class this PR patched five separate times, just
    // reached from the setter side instead of a missing clearer.
    // Gating here is the structural fix: refuse to set stale-by-
    // construction state instead of enumerating every possible way
    // to clear it after the fact.
    //
    // reagent (round 6, PLAUSIBLE): the round-5 guard above doesn't
    // catch a NARROWER race — a stale `compaction_started` ping can
    // also arrive AFTER its own matching `CompactionBoundary` while
    // the turn is STILL working (e.g. streaming new content past
    // the compaction that already completed), since `workingFromPhase`
    // stays true the whole time and doesn't distinguish "before" from
    // "after" the boundary. `compaction_started` and `compact_boundary`
    // travel over two independent transports (MPS vs. the primary
    // NDJSON stream) with no ordering guarantee between them. Reject
    // any start whose own timestamp is at or before the most recent
    // known boundary — a genuinely NEW compaction must be later than
    // the previous one's own completion.
    const isStaleVsLastBoundary =
        state.lastCompactionBoundaryAt != null && command.at <= state.lastCompactionBoundaryAt;
    if (state.lastEventMs == null || isStaleVsLastBoundary) {
        return { state, events: [] };
    }
    // SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md: a
    // ping can ALSO arrive too EARLY — before this pane's turnPhase
    // has been reconciled out of the mount-default Idle
    // (`ReconcileTurnActive` is an async RPC round-trip), or while
    // it's Disconnected, or Done.completed (a multi-round
    // tool-continuation's next round hasn't started yet). Every one
    // of those is a phase a LATER authoritative signal for THIS SAME
    // turn — `ReconcileTurnActive(active: true)` or
    // `StreamFlushObserved`'s own Idle/Disconnected/Done.completed
    // promotion arm — can still legitimately promote into Streaming.
    // Buffer instead of dropping so that promotion can retroactively
    // apply it (see those two cases below); this channel has no
    // replay, so dropping here is permanent for this compaction.
    // Two earlier attempts at this fix lived entirely in
    // `useCompactionStream.ts` (a local retry keyed off observing
    // `turnPhase`, then a time-bounded version of the same) — both
    // were structurally unsound: a hook observing only the
    // RESULTING `turnPhase` value cannot distinguish "nothing
    // happened" from "an authoritative inactive confirmation just
    // arrived and happened not to change the phase" (reviewer
    // findings, reagent P1 + codex P2 on PR #2928). Only the reducer
    // sees the discrete `ReconcileTurnActive(active: false)` command
    // itself regardless of whether it changes `turnPhase` — see that
    // case below, which clears this buffer explicitly on exactly
    // that signal instead of inferring it from a value that may
    // never change.
    //
    // Anything NOT in that promotable set (Done.errored/stopped/
    // interrupted, Interrupting) can never legitimately become
    // working again for this same turn — drop as before (round 5's
    // orphan-state guard: setting `compacting`, or now buffering,
    // against a phase nothing will ever promote out of just strands
    // it).
    if (!workingFromPhase(state.turnPhase)) {
        const canBecomeWorkingLater =
            state.turnPhase.kind === "Idle" ||
            state.turnPhase.kind === "Disconnected" ||
            (state.turnPhase.kind === "Done" && state.turnPhase.outcome === "completed");
        if (!canBecomeWorkingLater) {
            return { state, events: [] };
        }
        return {
            state: {
                ...state,
                pendingCompactionPing: { trigger: command.trigger, startedAt: command.at },
            },
            events: [{ type: "compaction-started-buffered", trigger: command.trigger }],
        };
    }
    const next: AgentPaneState = {
        ...state,
        lastEventMs: command.at,
        compacting: { trigger: command.trigger, startedAt: command.at },
        pendingCompactionPing: null,
    };
    if (next.turnPhase.kind === "Streaming") {
        next.turnPhase = { ...next.turnPhase, lastEventMs: command.at };
    }
    return {
        state: next,
        events: [{ type: "compaction-started", trigger: command.trigger }],
    };
}

/** `CompactionBoundary`. */
export function onCompactionBoundary(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "CompactionBoundary" }>): ReducerResult {
    // Codex P2 on PR #2378 (round 8): compact_boundary and
    // compaction_started travel over two independent transports
    // with no ordering guarantee (same root cause as the
    // CompactionStarted staleness guard above, mirrored here). If
    // compaction N+1 has already started (state.compacting) before
    // a DELAYED boundary for compaction N arrives, clearing
    // `compacting` unconditionally would wipe the genuinely
    // active N+1 state using stale N data. Only clear it when this
    // boundary's own completion time is at or after the
    // currently-tracked start — i.e. it's this same compaction (or
    // an even older one) finishing, not a stale echo racing a
    // newer one that's already begun. Falls back to clearing when
    // `frameTimestamp` is unavailable/unparseable — can't tell,
    // and a permanently-stuck "Compacting…" is worse than an
    // occasional early clear.
    const boundaryAt = command.frameTimestamp != null ? Date.parse(command.frameTimestamp) : NaN;
    const preservesNewerCompaction =
        state.compacting != null && !Number.isNaN(boundaryAt) && boundaryAt < state.compacting.startedAt;
    // SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md:
    // same "which one is this boundary actually for" reasoning as
    // `preservesNewerCompaction`, applied to a still-buffered
    // (never-promoted) ping. If this boundary is for that exact
    // buffered ping's compaction (or an even older one), the ping
    // is now moot — clear it, there's nothing left to retroactively
    // promote. If a NEWER ping is already buffered, preserve it —
    // this boundary can't be for a compaction that hasn't started
    // by this pane's own record yet.
    const preservesNewerPendingPing =
        state.pendingCompactionPing != null &&
        !Number.isNaN(boundaryAt) &&
        boundaryAt < state.pendingCompactionPing.startedAt;
    const next: AgentPaneState = {
        ...state,
        compacting: preservesNewerCompaction ? state.compacting : null,
        pendingCompactionPing: preservesNewerPendingPing ? state.pendingCompactionPing : null,
        // Codex P2 on PR #2378 (round 10): use this boundary's own
        // parsed completion time, not `command.at` (the frontend's
        // receipt wall-clock). `CompactionStarted.at` is the MPS
        // payload's embedded true start time, not a receipt
        // timestamp — comparing it against a delayed boundary's
        // RECEIPT time in the isStaleVsLastBoundary check above
        // means network/stream lag on this (older) boundary's
        // delivery can exceed a legitimately-earlier next
        // compaction's true start, falsely rejecting it. Falls
        // back to `command.at` when frameTimestamp is unparseable,
        // same as preservesNewerCompaction above.
        lastCompactionBoundaryAt: Number.isNaN(boundaryAt) ? command.at : boundaryAt,
        // The reading measured the pre-compaction conversation: none
        // until the next call. `postTokens` is not the new size (summary
        // messages only, no system prompt or tools; see context-reading.ts).
        // Kept for an older compaction finishing late, gated like
        // `compacting` (#2378), or a reading taken after the boundary.
        context:
            preservesNewerCompaction || readingPostdates(state.context, Number.isNaN(boundaryAt) ? command.at : boundaryAt)
                ? state.context
                : null,
        // A restored transcript ends before this boundary.
        contextSeedable: false,
    };
    if (state.lastEventMs != null) {
        next.lastEventMs = command.at;
        if (next.turnPhase.kind === "Streaming") {
            next.turnPhase = { ...next.turnPhase, lastEventMs: command.at };
        }
    }
    // codex P1 on PR #2659: a MANUAL compaction (the "Compact now"
    // button / a user typing "/compact") is sent through the normal
    // handleSendMessage path, which dispatches TurnStart the same as
    // any real conversational turn — but the CLI's actual response
    // to a standalone "/compact" is `system/status` + this boundary,
    // never an assistant text frame or a `result` frame. Those are
    // the ONLY two things claude-translator.ts's handleAssistantMessage
    // ever uses to fire session_end → TurnEnd. Without this, the pane
    // is left permanently "Working" after every successful manual
    // compaction, holding every subsequent message behind a turn
    // that will never naturally finish. (A FAILED manual compact
    // does NOT need this: the CLI's "Not enough messages to compact."
    // reply IS a real assistant text frame, so the existing
    // session_end path already closes it out correctly.)
    //
    // Gated on `state.pendingCompactTurn`, NOT `command.trigger ===
    // "manual"` alone (see that field's doc comment, types.ts) — a
    // manual trigger by itself doesn't prove THIS pane's currently
    // working turn is the synthetic one the compact action opened;
    // compaction_started/compact_boundary travel over independent
    // transports with no ordering guarantee against the primary
    // stream, and the reducer's own CompactionStarted race-guard
    // tests deliberately model a manual boundary landing on an
    // unrelated already-streaming turn.
    if (command.trigger === "manual" && state.pendingCompactTurn && workingFromPhase(next.turnPhase)) {
        // Mirrors TurnEnd's own outcome selection: a user who hit
        // Esc mid-compaction (Interrupting) gets "stopped", not a
        // misleading "completed" — same dual-write this reducer
        // already does for every other turn-ending case.
        const outcome: TurnOutcome = next.turnPhase.kind === "Interrupting" ? "stopped" : "completed";
        next.currentTool = null;
        next.currentToolArg = null;
        next.turnTokens = null;
        next.pendingCompactTurn = false;
        next.turnPhase = { kind: "Done", outcome, finishedAt: command.at };
    }
    return {
        state: next,
        events: [
            {
                type: "context-compacted",
                tokensBefore: command.preTokens,
                tokensAfter: command.postTokens,
                source: "real",
                trigger: command.trigger,
                durationMs: command.durationMs,
                frameTimestamp: command.frameTimestamp,
                boundaryUuid: command.boundaryUuid,
            },
        ],
    };
}
