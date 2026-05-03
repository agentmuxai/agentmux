// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for the agent-pane-state reducer (slice #4 in
 * docs/specs/frontend-reducer-architecture-2026-05-03.md).
 *
 * Bundles the per-pane atoms that have cohesive cross-atom invariants:
 *   streaming, sessionStats, currentTool, turnTokens, turnActive,
 *   stopping, pendingMessages.
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
}

export const initialState = (agentId: string): AgentPaneState => ({
    streaming: { active: false, agentId, bufferSize: 0, lastEventTime: 0 },
    sessionStats: null,
    currentTool: null,
    turnTokens: null,
    turnActive: false,
    stopping: false,
    pending: [],
});

export type AgentPaneCommand =
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
    | { type: "PendingMessageRejected"; id: string };

export type AgentPaneEvent =
    | { type: "stream-subscribed"; at: number }
    | { type: "stream-unsubscribed"; at: number }
    | { type: "stream-flush-observed"; addedCount: number }
    | { type: "turn-started"; at: number }
    | { type: "turn-ended"; statsMerged: boolean; stoppingCleared: boolean }
    | { type: "turn-reset" }
    | {
          /** Invariant fire: TurnStart while streaming inactive — dropped. */
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
    | { type: "pending-rejected"; id: string; wasPresent: boolean };

export interface ReducerResult {
    state: AgentPaneState;
    events: AgentPaneEvent[];
}
