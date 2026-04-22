// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * State management for agent widget using SolidJS signals
 *
 * IMPORTANT: All signals are instance-scoped (created per ViewModel instance)
 * to prevent state bleeding between multiple agent widgets.
 */

import { createSignal, type Accessor, type Setter } from "solid-js";
import {
    DocumentNode,
    DocumentState,
    SessionStats,
    StreamingState,
    TurnTokens,
} from "./types";

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
    };
}
