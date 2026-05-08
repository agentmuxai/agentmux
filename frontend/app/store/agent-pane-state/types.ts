// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for the agent-pane-state reducer (slice #4 in
 * docs/specs/frontend-reducer-architecture-2026-05-03.md).
 *
 * Bundles the per-pane atoms that have cohesive cross-atom invariants:
 *   streaming, sessionStats, currentTool, turnTokens, turnActive,
 *   stopping, pendingMessages, plus init phase and stream-watchdog fields
 *   added in issue #728.
 *
 * The agent document (message list) lives in its own slice
 * (`agent-document/`); reducer scope is intentionally separate per the
 * conventions doc §11 ("no god-reducer").
 */

import type { PendingMessage } from "../../view/agent/state";
import type {
    SessionStats,
    StreamingState,
    TurnTokens,
} from "../../view/agent/types";

/** Init lifecycle — covers the "history still loading" gap. */
export type InitPhase = "loading" | "ready" | "error";

/**
 * The reducer's state. Each field maps 1:1 to a Solid signal that the
 * agent pane projects from. The reducer enforces invariants ACROSS
 * fields (e.g. turnActive can't be true while streaming inactive).
 */
export interface AgentPaneState {
    streaming: StreamingState;
    sessionStats: SessionStats | null;
    currentTool: string | null;
    turnTokens: TurnTokens | null;
    turnActive: boolean;
    stopping: boolean;
    pending: PendingMessage[];
    /** Init phase — `loading` until history fetch completes (or fails). */
    initPhase: InitPhase;
    /** Reason captured on InitFailed; null otherwise. */
    initError: string | null;
    /**
     * Wall-clock ms of the last observable stream activity (subscribe,
     * flush, tool, tokens). Drives the stuck-stream watchdog. null
     * when no stream is active.
     */
    lastEventMs: number | null;
}

export const initialState = (agentId: string): AgentPaneState => ({
    streaming: { active: false, agentId, bufferSize: 0, lastEventTime: 0 },
    sessionStats: null,
    currentTool: null,
    turnTokens: null,
    turnActive: false,
    stopping: false,
    pending: [],
    initPhase: "loading",
    initError: null,
    lastEventMs: null,
});

export type AgentPaneCommand =
    // ── Init lifecycle (gap 1) ─────────────────────────────────────
    /** Caller signal: history fetch began. */
    | { type: "InitStart" }
    /** Caller signal: history fetch resolved successfully. */
    | { type: "InitReady" }
    /** Caller signal: history fetch failed; reason surfaced for diagnostics. */
    | { type: "InitFailed"; reason: string }

    // ── Stream lifecycle ───────────────────────────────────────────
    /** Hook signal: subscription is up. */
    | { type: "StreamSubscribe"; at: number }
    /** Hook signal: subscription torn down. */
    | { type: "StreamUnsubscribe"; at: number }
    /**
     * RAF flush observed by useAgentStream — bumps bufferSize +
     * lastEventTime. No state change beyond those counters.
     */
    | { type: "StreamFlushObserved"; addedCount: number; at: number }
    /**
     * Periodic tick from useAgentStream's watchdog interval. Emits a
     * `stream-stuck` event when the active stream has been silent for
     * more than `STUCK_THRESHOLD_MS`. No state mutation.
     */
    | { type: "StreamWatchdogTick"; nowMs: number }

    // ── Turn lifecycle ─────────────────────────────────────────────
    /**
     * User pressed send — turn becomes active. Also clears stale
     * sessionStats from the previous turn.
     */
    | { type: "TurnStart"; at: number }
    /**
     * Stream produced session_end (or fallback timer fired). Final
     * stats merged with current turn-tokens. Clears currentTool,
     * turnTokens, turnActive, AND stopping (the latter cascades to
     * "stop applied" without a separate command).
     */
    | { type: "TurnEnd"; stats: SessionStats | null }
    /**
     * Truncate path: the doc was reset (or whatever caused the local
     * stream-restart). Clears all per-turn state but does NOT touch
     * pending or streaming.
     */
    | { type: "TurnReset" }

    // ── Tool ───────────────────────────────────────────────────────
    | { type: "ToolStart"; name: string }
    | { type: "ToolEnd" }

    // ── Tokens (live deltas during a turn) ────────────────────────
    | { type: "TokensIn"; input: number }
    | { type: "TokensOut"; output: number }

    // ── Stop flow ──────────────────────────────────────────────────
    /** User pressed Esc / clicked Stop. */
    | { type: "RequestStop"; at: number }
    /** Stop RPC failed — bail on the stopping state. */
    | { type: "StopFailed" }

    // ── Pending message queue (composer side) ─────────────────────
    | { type: "PendingMessageQueued"; id: string; text: string; at: number }
    /** Backend acknowledged the message — remove from pending. */
    | { type: "PendingMessageAccepted"; id: string }
    /** RPC failed — remove the entry so user doesn't see a ghost row. */
    | { type: "PendingMessageRejected"; id: string }
    /**
     * Pending entry ageing watchdog fired (caller-scheduled setTimeout).
     * Removes the entry by id; idempotent no-op if already accepted/rejected.
     * Issue #728 gap 2 — prevents zombie pending rows when the backend
     * never acknowledges a queued message (e.g. RPC hang post-send).
     */
    | { type: "PendingMessageExpired"; id: string };

export type AgentPaneEvent =
    | { type: "init-started" }
    | { type: "init-ready" }
    | { type: "init-failed"; reason: string }
    | { type: "stream-subscribed"; at: number }
    | { type: "stream-unsubscribed"; at: number }
    | { type: "stream-flush-observed"; addedCount: number }
    | {
          /**
           * Stream has been silent for `idleSinceMs` while subscribed —
           * exceeds `thresholdMs`. Surfaced for diagnostics / UI badge.
           */
          type: "stream-stuck";
          idleSinceMs: number;
          thresholdMs: number;
      }
    | { type: "turn-started"; at: number }
    | { type: "turn-ended"; statsMerged: boolean; stoppingCleared: boolean }
    | { type: "turn-reset" }
    | {
          /**
           * Invariant fire: TurnStart while streaming inactive OR
           * initPhase !== "ready" — dropped.
           */
          type: "turn-start-suppressed";
          reason: string;
      }
    | { type: "tool-started"; name: string }
    | { type: "tool-ended" }
    | { type: "tokens-updated"; input: number | null; output: number | null }
    | { type: "stop-requested"; at: number }
    | { type: "stop-failed" }
    | { type: "pending-queued"; id: string }
    | { type: "pending-accepted"; id: string; wasPresent: boolean }
    | { type: "pending-rejected"; id: string; wasPresent: boolean }
    | {
          /** Pending entry expired and was removed; queuedAt for log/audit. */
          type: "pending-expired";
          id: string;
          queuedAt: number;
          ageMs: number;
          wasPresent: boolean;
      };

export interface ReducerResult {
    state: AgentPaneState;
    events: AgentPaneEvent[];
}

/**
 * Stuck-stream threshold. A subscribed stream that hasn't received any
 * observable event (subscribe, flush, tool, tokens) for this many ms is
 * flagged. 45s covers the typical Claude/Codex idle window between
 * thinking and tool calls during long reasoning runs without false
 * positives. Issue #728 gap 3.
 */
export const STUCK_THRESHOLD_MS = 45_000;
