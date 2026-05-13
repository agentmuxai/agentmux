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
import type { InitPhase } from "@/app/store/agent-pane-state/types";

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
    streamingStateAtom: SignalPair<StreamingState>;
    sessionStatsAtom: SignalPair<SessionStats | null>;
    currentToolAtom: SignalPair<string | null>;
    turnTokensAtom: SignalPair<TurnTokens | null>;
    /** True from the moment the user sends a message until session_end arrives. */
    turnActiveAtom: SignalPair<boolean>;
    /**
     * True from the moment the user presses Esc (stopAgent) until session_end
     * arrives. Drives the "Stopping…" label on the status line and triggers
     * an "⏹ Interrupted by user" chat row to be appended when the session
     * actually ends — so the user gets immediate acknowledgment *and*
     * confirmation that the interrupt landed.
     */
    stoppingAtom: SignalPair<boolean>;
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
     * Init lifecycle — `loading` until the initial history fetch resolves
     * (or fails). Drives a "Loading history…" hint and gates `TurnStart`.
     * Issue #728 gap 1.
     */
    initPhaseAtom: SignalPair<InitPhase>;
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
            active: false,
            agentId: agentId,
            bufferSize: 0,
            lastEventTime: 0,
        }),
        sessionStatsAtom: createSignal<SessionStats | null>(null),
        currentToolAtom: createSignal<string | null>(null),
        turnTokensAtom: createSignal<TurnTokens | null>(null),
        turnActiveAtom: createSignal<boolean>(false),
        stoppingAtom: createSignal<boolean>(false),
        pendingMessagesAtom: createSignal<PendingMessage[]>([]),
        initPhaseAtom: createSignal<InitPhase>("loading"),
    };
}
