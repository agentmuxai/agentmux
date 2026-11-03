// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the agent pane's lifecycle/turn/tool/tokens/
 * pending state. See docs/specs/agent-pane-document-reducer-2026-05-03.md
 * (slice #1 — pattern reference) and frontend-reducer-conventions-2026-05-03.md.
 *
 * Invariants enforced:
 *   1. A turn cannot enter Submitting (TurnStart) while the stream is
 *      unsubscribed (suppressed).
 *   2. A turn cannot enter Submitting while initPhase.kind ===
 *      "InitPending" (suppressed). After `InitFailed` we fail open —
 *      see the `TurnStart` arm.
 *   3. currentTool and turnTokens clear on TurnEnd / TurnReset.
 *   4. pending FIFO; accepting/rejecting/expiring unknown ids is no-op.
 *   5. StreamFlushObserved is a no-op when the stream is unsubscribed.
 *   6. StreamWatchdogTick is a no-op when the stream is unsubscribed or
 *      lastEventMs is null (no events seen yet); past LIVENESS_RECOVERY_MS it
 *      force-recovers a hung Streaming turn (no tool active) to Idle — and for a
 *      stalled rate-limit wait, past retryAfterMs + LIVENESS_RECOVERY_MS.
 *   7. Init lifecycle transitions are one-way: InitStart / InitReady /
 *      InitFailed are no-ops once the pane has reached a terminal init
 *      state (InitReady / InitFailed). Re-entering pending requires a
 *      fresh pane slot.
 *
 * Stream-subscription gate: since PR G the legacy `streaming.active`
 * boolean is gone. `state.lastEventMs !== null` is the canonical "is
 * the stream subscribed?" check — both fields always moved in lockstep
 * (only StreamSubscribe / StreamUnsubscribe ever touched either) so
 * the substitution is mechanical. The {@link isStreamSubscribed} selector
 * wraps the check.
 */

import type { AgentPaneCommand, AgentPaneState, ReducerResult } from "./types";
import { onInitStart, onInitReady, onInitFailed, onStreamSubscribe, onStreamUnsubscribe, onStreamFlushObserved, onStreamWatchdogTick, onReconcileTurnActive } from "./reducer-lifecycle";
import { onReconcileContextFromHistory, onContextWindowsReported, onContextReported, onContextInvalidated, onContextModelSwitched } from "./reducer-context";
import { onTurnStart, onTurnEnd, onStreamSessionStarted, onTurnReset, onTurnObserved, onTurnStartFailed } from "./reducer-turn";
import { onToolStart, onToolEnd, onTokensIn, onModelEndedTurn, onTokensOut, onToolProgress, onToolOutput, onThinkingHeadline, onOutputStreamed } from "./reducer-tools";
import { onRequestStarted, onRequestStop, onStopFailed, onInterruptTimeoutElapsed, onSubmitTimeoutElapsed } from "./reducer-stop";
import { onPendingMessageFlushStarted, onPendingMessageQueued, onPendingMessageAccepted, onPendingMessageRejected, onPendingMessageExpired, onProviderWaiting } from "./reducer-pending";
import { onFailureObserved, onFailureCleared, onCompactionStarted, onCompactionBoundary } from "./reducer-failure";
import { onDetailsToggle, onDetailsExpand, onDetailsCollapse, onAttachedTaskObserved, onAttachedTaskCleared, onRegistryAttachedTaskObserved, onRegistryAttachedTaskCleared, onResumeRetryStarted, onResumeRetryResolved } from "./reducer-axes";

/** What handles each command: one function per command, so the type checker names a command missing here. */
type Handlers = {
    [K in AgentPaneCommand["type"]]: (state: AgentPaneState, command: Extract<AgentPaneCommand, { type: K }>, nowMs: number) => ReducerResult;
};

const HANDLERS: Handlers = {
    InitStart: onInitStart,
    InitReady: onInitReady,
    InitFailed: onInitFailed,
    StreamSubscribe: onStreamSubscribe,
    StreamUnsubscribe: onStreamUnsubscribe,
    StreamFlushObserved: onStreamFlushObserved,
    StreamWatchdogTick: onStreamWatchdogTick,
    ReconcileTurnActive: onReconcileTurnActive,
    ReconcileContextFromHistory: onReconcileContextFromHistory,
    ContextWindowsReported: onContextWindowsReported,
    ContextReported: onContextReported,
    ContextInvalidated: onContextInvalidated,
    ContextModelSwitched: onContextModelSwitched,
    TurnStart: onTurnStart,
    TurnEnd: onTurnEnd,
    StreamSessionStarted: onStreamSessionStarted,
    TurnReset: onTurnReset,
    TurnObserved: onTurnObserved,
    TurnStartFailed: onTurnStartFailed,
    ToolStart: onToolStart,
    ToolEnd: onToolEnd,
    TokensIn: onTokensIn,
    ModelEndedTurn: onModelEndedTurn,
    ModelMessageStarted: onModelEndedTurn,
    TokensOut: onTokensOut,
    ToolProgress: onToolProgress,
    ToolOutput: onToolOutput,
    ThinkingHeadline: onThinkingHeadline,
    OutputStreamed: onOutputStreamed,
    RequestStarted: onRequestStarted,
    RequestStop: onRequestStop,
    StopFailed: onStopFailed,
    InterruptTimeoutElapsed: onInterruptTimeoutElapsed,
    SubmitTimeoutElapsed: onSubmitTimeoutElapsed,
    PendingMessageFlushStarted: onPendingMessageFlushStarted,
    PendingMessageQueued: onPendingMessageQueued,
    PendingMessageAccepted: onPendingMessageAccepted,
    PendingMessageRejected: onPendingMessageRejected,
    PendingMessageExpired: onPendingMessageExpired,
    ProviderWaiting: onProviderWaiting,
    FailureObserved: onFailureObserved,
    FailureCleared: onFailureCleared,
    CompactionStarted: onCompactionStarted,
    CompactionBoundary: onCompactionBoundary,
    DetailsToggle: onDetailsToggle,
    DetailsExpand: onDetailsExpand,
    DetailsCollapse: onDetailsCollapse,
    AttachedTaskObserved: onAttachedTaskObserved,
    AttachedTaskCleared: onAttachedTaskCleared,
    RegistryAttachedTaskObserved: onRegistryAttachedTaskObserved,
    RegistryAttachedTaskCleared: onRegistryAttachedTaskCleared,
    ResumeRetryStarted: onResumeRetryStarted,
    ResumeRetryResolved: onResumeRetryResolved,
};

export function update(
    state: AgentPaneState,
    command: AgentPaneCommand,
    /**
     * Time injection for testability + watchdog determinism. Used by
     * tool/token transitions to bump `lastEventMs` (the stuck-stream
     * watchdog clock). Defaults to `Date.now()` so existing callers
     * don't need to thread a timestamp.
     */
    nowMs: number = Date.now(),
): ReducerResult {
    const handle = HANDLERS[command.type] as (s: AgentPaneState, c: AgentPaneCommand, n: number) => ReducerResult;
    return handle(state, command, nowMs);
}
