// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AgentPaneCommand, AgentPaneEvent, AgentPaneState, INTERRUPT_TIMEOUT_MS, ReducerResult, TurnPhase } from "./types";
import { withPhase, bumpEvent } from "./reducer-helpers";

/** `RequestStarted`. */
export function onRequestStarted(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "RequestStarted" }>, nowMs: number): ReducerResult {
    const t = state.turnTokens;
    if (!t) return { state, events: [] };
    const activity = withPhase(state.activity, "requesting", nowMs);
    if (t.requesting) return activity === state.activity ? { state, events: [] } : { state: { ...state, activity }, events: [] };
    return { state: { ...state, turnTokens: { ...t, requesting: true }, activity }, events: [] };
}

/** `RequestStop`. */
export function onRequestStop(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "RequestStop" }>): ReducerResult {
    // If a turn is in flight, surface Interrupting; otherwise
    // the stop is a no-op (no working state to interrupt).
    // PR G removed the legacy `stopping` boolean — there is no
    // hidden "stop pending" mode outside the working set.
    const k = state.turnPhase.kind;
    const isWorking =
        k === "Submitting" || k === "Streaming" || k === "Interrupting";
    if (!isWorking) {
        // No-op stop — emit `stop-requested` for diagnostic
        // continuity but leave state untouched. The legacy
        // boolean used to flip true here even with no turn in
        // flight; that was a latent bug surface (the stop
        // could not actually be acted on) and PR G drops it.
        return {
            state,
            events: [{ type: "stop-requested", at: command.at }],
        };
    }
    // PR C: only schedule the bounded-interrupt watchdog when we
    // actually CROSS INTO Interrupting (not when we're already
    // there — that would double-arm the timeout). Spec §8: SIGINT
    // is only emitted once on entry; the timeout follows the same
    // rule so a second Stop press doesn't reset the deadline.
    const enteringInterrupting = k !== "Interrupting";
    // Codex P2 on #991: preserve the original sigintSentAt on a
    // repeated Stop press. The timeout was armed from the FIRST
    // press's deadline; rebuilding the phase with a fresh
    // `command.at` here makes consumers see a `sigintSentAt`
    // that doesn't match the active deadline. When we're
    // already Interrupting, reuse the existing phase so the
    // timestamp stays pinned to the original SIGINT.
    const nextPhase: TurnPhase =
        k === "Interrupting"
            ? state.turnPhase
            : {
                  kind: "Interrupting",
                  reason: "user",
                  sigintSentAt: command.at,
              };
    const events: AgentPaneEvent[] = [
        { type: "stop-requested", at: command.at },
    ];
    if (enteringInterrupting) {
        events.push({
            type: "schedule-interrupt-timeout",
            deadlineMs: command.at + INTERRUPT_TIMEOUT_MS,
        });
    }
    return {
        // Codex P2 on PR #2378 (round 3): deliberately does NOT
        // clear `compacting` here (an earlier version of this fix
        // did, per a since-superseded reagent finding). RequestStop
        // only SENDS a SIGINT — it doesn't confirm the turn
        // actually ended. If the stop fails, `StopFailed` rolls
        // the phase back to Streaming with no way to know whether
        // compaction resumed unaffected the whole time (the SIGINT
        // never landed) — eagerly clearing here would have
        // silently dropped the "Compacting…" status and its timer
        // for a compaction that was never actually interrupted.
        // TurnEnd/TurnReset/StreamUnsubscribe/FailureObserved
        // already clear `compacting` on every path that actually,
        // authoritatively ends the turn (whether the stop
        // succeeded or the turn ended some other way) — that's
        // sufficient; this transition doesn't need its own clear.
        state: { ...state, turnPhase: nextPhase },
        events,
    };
}

/** `StopFailed`. */
export function onStopFailed(state: AgentPaneState): ReducerResult {
    // Rollback Interrupting → previous working kind best-effort.
    // We don't track the prior kind on Interrupting, so use
    // the stream-subscribed gate as the deterministic signal:
    //   subscribed (lastEventMs !== null) → Streaming (common case)
    //   else                              → Idle (no longer in a turn)
    // Non-Interrupting phases are a no-op — there's nothing
    // to roll back. PR G dropped the legacy `stopping` boolean
    // that previously also cleared here.
    const k = state.turnPhase.kind;
    const nextPhase: TurnPhase =
        k === "Interrupting"
            ? state.lastEventMs !== null
                ? {
                      kind: "Streaming",
                      bufferSize: state.streaming.bufferSize,
                      toolsActive: 0,
                      lastEventMs: state.lastEventMs,
                  }
                : { kind: "Idle" }
            : state.turnPhase;
    return {
        state: { ...state, turnPhase: nextPhase },
        events: [{ type: "stop-failed" }],
    };
}

/** `InterruptTimeoutElapsed`. */
export function onInterruptTimeoutElapsed(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "InterruptTimeoutElapsed" }>): ReducerResult {
    // PR C — bounded `Interrupting → Done.interrupted`.
    //
    // The dispatch side schedules a setTimeout when it sees the
    // `schedule-interrupt-timeout` event. A shared cancel flag
    // (the JS analogue of `Arc<AtomicBool>`) prevents a stale
    // timer from firing after a graceful TurnEnd / unsubscribe
    // already moved us off Interrupting — but defence in depth:
    // the reducer ALSO checks the current phase and treats a
    // late tick as a no-op. Three independent paths into
    // Done.interrupted (TurnEnd, StreamUnsubscribe → Disconnected,
    // and this timeout) — so the working animation can never
    // hang on Interrupting indefinitely. Spec §8.
    if (state.turnPhase.kind !== "Interrupting") {
        return { state, events: [] };
    }
    return {
        state: {
            ...state,
            currentTool: null,
            currentToolArg: null,
            turnTokens: null,
            turnPhase: {
                kind: "Done",
                outcome: "interrupted",
                finishedAt: command.at,
            },
            // reagent + codex P1 on PR #2378 (round 4): a fifth
            // authoritative terminal transition, same bug class as
            // the four already fixed. Round 3 deliberately stopped
            // RequestStop from clearing `compacting` (so it survives
            // a failed stop attempt) — but if the interrupt instead
            // TIMES OUT here, that IS an authoritative end of the
            // turn, same as TurnEnd/TurnReset/StreamUnsubscribe/
            // FailureObserved, and must clear it the same way.
            compacting: null,
            pendingCompactionPing: null,
        },
        events: [{ type: "interrupt-timed-out", at: command.at }],
    };
}

/** `SubmitTimeoutElapsed`. */
export function onSubmitTimeoutElapsed(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "SubmitTimeoutElapsed" }>): ReducerResult {
    // PR D — bounded `Submitting → Done.errored`.
    //
    // The dispatch side schedules a setTimeout when it sees the
    // `schedule-submit-timeout` event emitted by `TurnStart`. A
    // shared cancel flag (the JS analogue of `Arc<AtomicBool>`,
    // same pattern as the PR C interrupt timer) prevents a stale
    // timer from firing after a graceful promotion to Streaming
    // (via `StreamFlushObserved` or `bumpEvent` from a tool /
    // token event). Defence in depth: the reducer ALSO checks
    // the current phase and treats a late tick as a no-op.
    //
    // Three independent paths out of Submitting — promotion via
    // `StreamFlushObserved`, promotion via `bumpEvent` (tool /
    // token), and this timeout — so the working animation can
    // never get stuck on Submitting indefinitely. The first
    // promotion wins; this timeout fires only if none arrived.
    // Spec §8 / issue #728 gap 2.
    if (state.turnPhase.kind !== "Submitting") {
        return { state, events: [] };
    }
    return {
        state: {
            ...state,
            currentTool: null,
            currentToolArg: null,
            turnTokens: null,
            turnPhase: {
                kind: "Done",
                outcome: "errored",
                finishedAt: command.at,
            },
            // Same reasoning as InterruptTimeoutElapsed above:
            // realistically compaction only happens once Streaming
            // has started, so `compacting` shouldn't be set while
            // still Submitting — but CompactionStarted's own
            // handling doesn't gate on phase kind, so clear it here
            // too for the same defensive completeness reagent/codex
            // asked for on the other bounded-timeout arm.
            compacting: null,
            pendingCompactionPing: null,
        },
        events: [{ type: "submit-timed-out", at: command.at }],
    };
}
