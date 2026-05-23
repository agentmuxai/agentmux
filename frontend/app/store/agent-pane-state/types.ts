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

// ─────────────────────────────────────────────────────────────────────────
// TurnPhase discriminated union (PR A — dual-write phase)
//
// SPEC: docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §5.
// This union replaces the scattered { turnActive, stopping, streaming.active }
// booleans with a single explicit phase. PR A is ADDITIVE: the legacy
// booleans stay and the reducer dual-writes both. View migration is PR B;
// the legacy fields are removed in PR G.
// ─────────────────────────────────────────────────────────────────────────

/** Why the turn entered Interrupting. */
export type InterruptReason =
    /** User pressed Esc / clicked Stop. */
    | "user"
    /** Caller-initiated (auto-stop, watchdog, etc.). */
    | "system";

/** Why the turn finished (kind: "Done"). */
export type TurnOutcome =
    /** Backend emitted session_end normally. */
    | "completed"
    /** Stop was requested before completion (user or system). */
    | "stopped"
    /** Stream torn down mid-turn — no graceful end. */
    | "interrupted"
    /** A non-zero turn ended without backend output (RPC fail, etc.). */
    | "errored";

/** The working-kind we were in before the stream dropped. */
export type KindBeforeDisconnect =
    | "Submitting"
    | "Streaming"
    | "Interrupting";

/**
 * Single source of truth for the turn lifecycle. PR A introduces this
 * alongside the legacy booleans — both are written by the reducer.
 *
 *   Idle          : no turn in flight.
 *   Submitting    : user pressed send; awaiting `StreamSubscribe` / first
 *                   stream event.
 *   Streaming     : stream is actively producing events.
 *   Interrupting  : stop requested; awaiting graceful TurnEnd or unsub.
 *   Done          : turn finished (with outcome).
 *   Disconnected  : stream dropped while a turn was in-flight; remembers
 *                   the kind we lost so the view can show a re-attach UI.
 */
export type TurnPhase =
    | { kind: "Idle" }
    | { kind: "Submitting"; submittedAt: number; pendingContent: string }
    | {
          kind: "Streaming";
          bufferSize: number;
          toolsActive: number;
          lastEventMs: number;
      }
    | {
          kind: "Interrupting";
          reason: InterruptReason;
          sigintSentAt: number;
      }
    | { kind: "Done"; outcome: TurnOutcome; finishedAt: number }
    | {
          kind: "Disconnected";
          lastKind: KindBeforeDisconnect;
          reason: string;
      };

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
    /**
     * PR A (dual-write): the single-source-of-truth turn phase. The
     * reducer writes BOTH this and the legacy {turnActive, stopping,
     * streaming.active} fields on every command. Subsequent PRs (B
     * onward) migrate view code off the legacy booleans onto this
     * field; PR G removes the booleans.
     */
    turnPhase: TurnPhase;
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
    turnPhase: { kind: "Idle" },
});

/**
 * Selector — true while the agent is processing a user request.
 *
 * Returns true ⇔ `state.turnPhase.kind ∈ {Submitting, Streaming, Interrupting}`.
 * Idle / Done / Disconnected are all "not working".
 *
 * Spec: SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §7. PR A exports this
 * for downstream consumers; the view migration in PR B will rebind the
 * footer's working indicator to this selector.
 */
export function isWorking(state: AgentPaneState): boolean {
    return workingFromPhase(state.turnPhase);
}

/**
 * Phase-only variant of {@link isWorking}. The view layer projects
 * `turnPhase` through a dedicated signal (PR B); call sites that only
 * have the phase (not the whole state) use this helper. Same predicate.
 *
 * Returns true ⇔ `phase.kind ∈ {Submitting, Streaming, Interrupting}`.
 */
export function workingFromPhase(phase: TurnPhase): boolean {
    const k = phase.kind;
    return k === "Submitting" || k === "Streaming" || k === "Interrupting";
}

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
    /**
     * Interrupt timeout watchdog fired (caller-scheduled setTimeout — see
     * `ScheduleInterruptTimeout` event). Bounded `Interrupting → Done`
     * force-transition: if the agent never acks the SIGINT (backend died,
     * stream dropped, etc.) the pane was previously stuck in Interrupting
     * forever and the working animation would never settle. This command
     * is a no-op if the phase has already moved off Interrupting
     * (e.g. agent acked first, user already resumed). Spec §8.
     */
    | { type: "InterruptTimeoutElapsed"; at: number }

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
    /**
     * Reducer signal: a turn has just entered Interrupting. The
     * dispatch layer (view / saga) is expected to start a setTimeout
     * for `INTERRUPT_TIMEOUT_MS` and fire `InterruptTimeoutElapsed`
     * when it expires — gated by a shared cancel flag (analogous to
     * `Arc<AtomicBool>`) so a later `RequestStop` doesn't double-fire.
     * `deadlineMs` is `at + INTERRUPT_TIMEOUT_MS` so the caller can
     * compute the remaining slice deterministically. Spec §8.
     */
    | {
          type: "schedule-interrupt-timeout";
          deadlineMs: number;
      }
    /**
     * Reducer signal: the bounded `Interrupting → Done.{interrupted}`
     * force-transition fired because no graceful ack arrived within
     * `INTERRUPT_TIMEOUT_MS`. The view can surface this as
     * "interrupt timed out — assuming dead" if needed. Spec §8.
     */
    | { type: "interrupt-timed-out"; at: number }
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

/**
 * Bounded `Interrupting → Done.{interrupted}` window. After `RequestStop`
 * fires SIGINT, if neither `TurnEnd` nor `StreamUnsubscribe` lands within
 * this many ms the reducer force-transitions to Done.interrupted on
 * receipt of `InterruptTimeoutElapsed`. 5 s is generous compared to the
 * typical sub-second SIGINT-ack on a healthy agent but tight enough that
 * a dead agent doesn't keep the working animation pinned forever.
 *
 * Spec: SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §8 — the spec doc
 * suggests 3_000 but the implementation lands at 5_000 to leave more
 * headroom for slow Windows PTY teardowns and remote agents on flaky
 * networks. Tune in a follow-up if telemetry shows the watchdog firing
 * on live agents.
 */
export const INTERRUPT_TIMEOUT_MS = 5_000;
