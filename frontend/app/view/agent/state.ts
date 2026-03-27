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
    AgentProcessState,
    AuthState,
    DocumentNode,
    DocumentState,
    MessageRouterState,
    StreamingState,
    UserInfo,
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
    processAtom: SignalPair<AgentProcessState>;
    messageRouterAtom: SignalPair<MessageRouterState>;
    authAtom: SignalPair<AuthState>;
    userInfoAtom: SignalPair<UserInfo | null>;
    providerConfigAtom: SignalPair<ProviderConfig | null>;
    sessionIdAtom: SignalPair<string>;
    rawOutputAtom: SignalPair<string>;
}

/**
 * Factory function: Create fresh signals for a new agent widget instance
 */
export function createAgentAtoms(agentId: string): AgentAtoms {
    return {
        documentAtom: createSignal<DocumentNode[]>([]),
        documentStateAtom: createSignal<DocumentState>({
            collapsedNodes: new Set<string>(),
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
        processAtom: createSignal<AgentProcessState>({
            pid: undefined,
            agentId: agentId,
            status: "idle",
            canRestart: true,
            canKill: false,
        }),
        messageRouterAtom: createSignal<MessageRouterState>({
            backend: "local",
            connected: false,
            endpoint: "",
        }),
        authAtom: createSignal<AuthState>({
            status: "disconnected",
        }),
        userInfoAtom: createSignal<UserInfo | null>(null),
        providerConfigAtom: createSignal<ProviderConfig | null>(null),
        sessionIdAtom: createSignal<string>(""),
        rawOutputAtom: createSignal<string>(""),
    };
}
