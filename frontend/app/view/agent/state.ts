// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * State management for agent widget using SolidJS signals.
 *
 * IMPORTANT: All signals are instance-scoped (created per ViewModel
 * instance) to prevent state bleeding between multiple agent widgets.
 *
 * **Architecture: this file is the read/render projection layer, not
 * a parallel state store.** The setters returned here are consumed by
 * `registerAgentPaneStatePane` and `registerAgentDocPane` on mount; the
 * slot-store reducers are the *only* callers of those setters. The
 * components in this directory read via the accessors and never touch
 * the setters directly. Every write to these signals therefore flows
 * through `dispatchPane` / `dispatchDoc` and is captured by the
 * `recordDispatch` audit ring.
 *
 * Anti-pattern to flag in review: a `set*` call from a component or
 * hook outside `state.test.ts`. That bypass would break replay (see
 * `docs/specs/SPEC_AGENT_PANE_SESSION_REPLAY_2026_05_12.md`) and the
 * audit confirmation in `docs/analysis/AGENT_PANE_REDUCER_AUDIT_2026_05_12.md`.
 */

import { createSignal, type Accessor, type Setter } from "solid-js";
import {
    DocumentNode,
    DocumentState,
    SessionStats,
    StreamingState,
    TurnTokens,
} from "./types";
import type { AttachedTaskState, CompactionState, InitPhase, PaneFailure, ResumeRetryState, TurnPhase } from "@/app/store/agent-pane-state/types";

/**
 * A signal pair: [getter, setter]
 */
export type SignalPair<T> = [Accessor<T>, Setter<T>];

/**
 * Collection of signals for a single agent widget instance
 */
export interface AgentAtoms {
    documentAtom: SignalPair<DocumentNode[]>;
    documentStateAtom: SignalPair<DocumentState>;
    /**
     * Telemetry sub-object — `bufferSize`, `lastEventTime`, `agentId`.
     * Write-only from the view layer's perspective (no current
     * consumer reads it); the reducer maintains it for buffer
     * accounting + the stuck-stream watchdog. The legacy `active`
     * boolean was dropped in PR G — "is the stream subscribed?" is
     * derived from `lastEventMs !== null` on the reducer state.
     */
    streamingStateAtom: SignalPair<StreamingState>;
    sessionStatsAtom: SignalPair<SessionStats | null>;
    /**
     * Cumulative cost/tokens/duration across every completed turn this
     * pane's lifetime — feeds the composer-strip totals display. See
     * SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md.
     */
    sessionTotalsAtom: SignalPair<SessionStats | null>;
    currentToolAtom: SignalPair<string | null>;
    turnTokensAtom: SignalPair<TurnTokens | null>;
    /**
     * Total input-token count from the last message_start event — equals
     * the full context fill (all conversation history) sent to the model
     * on that turn. Persists through TurnEnd; clears on TurnReset (session wipe).
     */
    contextTokensAtom: SignalPair<number | null>;
    /** Learned context-window for the current model (null → provider fallback). */
    contextWindowAtom: SignalPair<number | null>;
    /**
     * Messages sent by the user that the backend hasn't picked up yet.
     * Rendered in a pending zone between the conversation and the composer.
     * When the backend emits `agent-message-accepted` for a given id, the
     * frontend removes the entry from here and appends a normal `user_message`
     * node to the conversation document — that color shift is the visual
     * "accepted" signal. FIFO, matches the backend's `VecDeque` queue.
     */
    pendingMessagesAtom: SignalPair<PendingMessage[]>;
    /**
     * Init lifecycle — `InitPending` until the initial history fetch
     * resolves into `InitReady` (success) or `InitFailed` (error).
     * Drives a "Loading history…" hint and gates `TurnStart`. Issue
     * #728 gap 1; discriminated-union shape.
     */
    initPhaseAtom: SignalPair<InitPhase>;
    /**
     * Single-source-of-truth turn phase. PR A introduced TurnPhase
     * with dual-write against the legacy `turnActive` / `stopping` /
     * `streaming.active` fields, PR B migrated the view onto the
     * `isWorking(state)` / `state.turnPhase.kind` selectors, and
     * PR G removed the legacy fields entirely.
     *
     * Spec: docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §5–§7.
     */
    turnPhaseAtom: SignalPair<TurnPhase>;
    /**
     * Composer details panel — open/closed. Reducer-owned (see
     * `AgentPaneState.detailsOpen`, added in #1068). Drives the
     * chevron orientation in the composer strip and the conditional
     * render of the `AgentComposerDetails` block.
     *
     * SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26.md §5.4.
     */
    detailsOpenAtom: SignalPair<boolean>;
    /**
     * First significant argument of the active tool (file path, command, etc.).
     * Reducer-owned; cleared alongside `currentToolAtom` on ToolEnd / TurnEnd.
     * Drives the enriched `AgentWorkingRow` display.
     */
    currentToolArgAtom: SignalPair<string | null>;
    /**
     * Active classified failure for this pane, or null. Reducer-owned
     * (see `AgentPaneState.failure`) — replaces the local `failure` signal
     * `useAgentFailure.ts` used to hold on its own, with no path back into
     * `turnPhase`. See SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md.
     */
    failureAtom: SignalPair<PaneFailure | null>;
    /**
     * Live "compaction in progress" state, or null. Reducer-owned (see
     * `AgentPaneState.compacting`). Drives the "Compacting…" status chip
     * + elapsed counter in `AgentComposerStrip`. See
     * docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md.
     */
    compactingAtom: SignalPair<CompactionState | null>;
    /**
     * Live "≥1 agent-declared long-running activity attached" state, or
     * null. Reducer-owned (see `AgentPaneState.attachedTask`). Drives the
     * "Running…" footer status once `turnPhase` is otherwise idle — see
     * docs/specs/SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md.
     */
    attachedTaskAtom: SignalPair<AttachedTaskState | null>;
    /**
     * Live registry-derived attached-task floor, or null. Reducer-owned
     * (see `AgentPaneState.registryAttachedTaskSince`) — a SEPARATE axis
     * from `attachedTaskAtom` above, combined with it (not merged into it)
     * by `agent-view.tsx`'s attached-task effect. See
     * docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md.
     */
    registryAttachedTaskSinceAtom: SignalPair<number | null>;
    /**
     * Live "reconnecting after a stale `--resume` session id" state, or
     * null. Reducer-owned (see `AgentPaneState.reconnecting`). Drives the
     * "Reconnecting…" status chip + elapsed counter in `AgentComposerStrip`,
     * mirroring `compactingAtom`. See
     * docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md §6.2.
     */
    reconnectingAtom: SignalPair<ResumeRetryState | null>;
}

/**
 * Message held on the frontend while waiting for the backend to pick it up.
 * `id` is generated client-side and round-tripped through `AgentInputCommand`
 * so the `agent-message-accepted` event can match it.
 */
export interface PendingMessage {
    id: string;
    text: string;
    createdAt: number;
    /**
     * True when this message was queued while a turn was already in-flight
     * (Submitting | Streaming | Interrupting). False for messages that
     * initiated the current turn (idle sends).
     *
     * The PendingMessagesPanel gates its visibility on this flag so that
     * idle-send messages never flash in the amber queued zone — only messages
     * genuinely sitting behind a running turn should appear there.
     * See docs/analysis/ANALYSIS_IDLE_SEND_RACE_2026_06_11.md.
     */
    enqueuedWhileBusy: boolean;
    /**
     * Set by the `PendingMessageFlushStarted` reducer arm, dispatched from
     * `flushHeldMessages` (`useAgentCommands.ts`) right before its actual
     * `deliverToBackend` call — the moment delivery genuinely begins, not
     * merely "the turn this message was queued behind has ended." Those two
     * moments can be far apart when a controller refresh is still deferred
     * (`flushHeldMessages` bails without draining `heldQueue` in that case);
     * an earlier version of this flag was set unconditionally on `TurnEnd`
     * and could read `flushing` while delivery hadn't actually started yet
     * (codex P2 on PR #2970). Once set, the backend's async
     * `agent-message-accepted` ack (which is what actually removes this
     * entry, via `usePendingMessageAcceptance.ts`) or a delivery failure
     * (`PendingMessageRejected`) is the only thing left to wait for. Before
     * this flag existed, the panel had no way to distinguish "still behind a
     * running turn" from "turn ended, delivery in flight" — it read the same
     * "Queued — sends at the agent's next step" copy in both cases, which is
     * actively wrong once the turn is Done (there is no "next step" left to
     * wait for) and is what let "Queued message" visibly coexist with
     * "Worked" for the length of that ack round-trip. See
     * docs/specs/SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04.md Phase 1.
     */
    flushing?: boolean;
}

/**
 * Factory function: Create fresh signals for a new agent widget instance
 */
export function createAgentAtoms(agentId: string): AgentAtoms {
    return {
        documentAtom: createSignal<DocumentNode[]>([]),
        documentStateAtom: createSignal<DocumentState>({
            collapsedNodes: new Set<string>(),
            pinnedNodes: new Set<string>(),
            expandedTools: new Set<string>(),
            scrollPosition: 0,
            selectedNode: null,
            filter: {
                showThinking: false,
                showSuccessfulTools: true,
                showFailedTools: true,
                showIncoming: true,
                showOutgoing: true,
            },
        }),
        streamingStateAtom: createSignal<StreamingState>({
            agentId: agentId,
            bufferSize: 0,
            lastEventTime: 0,
        }),
        sessionStatsAtom: createSignal<SessionStats | null>(null),
        sessionTotalsAtom: createSignal<SessionStats | null>(null),
        currentToolAtom: createSignal<string | null>(null),
        turnTokensAtom: createSignal<TurnTokens | null>(null),
        contextTokensAtom: createSignal<number | null>(null),
        contextWindowAtom: createSignal<number | null>(null),
        pendingMessagesAtom: createSignal<PendingMessage[]>([]),
        // gap1 (#993) reshaped InitPhase from string union to discriminated
        // union; turnPhase from PR B is now the sole working-state encoding
        // after PR G removed the legacy `turnActiveAtom` / `stoppingAtom`.
        initPhaseAtom: createSignal<InitPhase>({ kind: "InitPending" }),
        turnPhaseAtom: createSignal<TurnPhase>({ kind: "Idle" }),
        // Composer details panel — reducer-owned.
        // SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26.md §5.4.
        detailsOpenAtom: createSignal<boolean>(false),
        currentToolArgAtom: createSignal<string | null>(null),
        failureAtom: createSignal<PaneFailure | null>(null),
        compactingAtom: createSignal<CompactionState | null>(null),
        attachedTaskAtom: createSignal<AttachedTaskState | null>(null),
        registryAttachedTaskSinceAtom: createSignal<number | null>(null),
        reconnectingAtom: createSignal<ResumeRetryState | null>(null),
    };
}
