// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * State management for agent widget using Jotai atoms
 *
 * IMPORTANT: All atoms are instance-scoped (created per ViewModel instance)
 * to prevent state bleeding between multiple agent widgets.
 */

import { atom, Atom, PrimitiveAtom, WritableAtom } from "jotai";
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
 * Collection of atoms for a single agent widget instance
 */
export interface AgentAtoms {
    documentAtom: PrimitiveAtom<DocumentNode[]>;
    documentStateAtom: PrimitiveAtom<DocumentState>;
    streamingStateAtom: PrimitiveAtom<StreamingState>;
    processAtom: PrimitiveAtom<AgentProcessState>;
    messageRouterAtom: PrimitiveAtom<MessageRouterState>;
    authAtom: PrimitiveAtom<AuthState>;
    userInfoAtom: PrimitiveAtom<UserInfo | null>;
    providerConfigAtom: PrimitiveAtom<ProviderConfig | null>;
    sessionIdAtom: PrimitiveAtom<string>;  // CLI session ID from system.init
    rawOutputAtom: PrimitiveAtom<string>;  // Raw CLI output (when translation is disabled)
}

/**
 * Factory function: Create fresh atoms for a new agent widget instance
 *
 * Each widget instance gets its own independent set of atoms.
 * This prevents state bleeding between multiple agent widgets.
 *
 * @param agentId - Unique identifier for this agent instance
 * @returns AgentAtoms object containing all instance-scoped atoms
 */
export function createAgentAtoms(agentId: string): AgentAtoms {
    return {
        documentAtom: atom<DocumentNode[]>([]),
        documentStateAtom: atom<DocumentState>({
            collapsedNodes: new Set<string>(),
            scrollPosition: 0,
            selectedNode: null,
            filter: {
                showThinking: false, // Hide thinking by default
                showSuccessfulTools: true, // Show successful tools
                showFailedTools: true, // Always show failures
                showIncoming: true, // Show incoming messages
                showOutgoing: true, // Show outgoing messages
            },
        }),
        streamingStateAtom: atom<StreamingState>({
            active: false,
            agentId: agentId,
            bufferSize: 0,
            lastEventTime: 0,
        }),
        processAtom: atom<AgentProcessState>({
            pid: undefined,
            agentId: agentId,
            status: "idle",
            canRestart: true,
            canKill: false,
        }),
        messageRouterAtom: atom<MessageRouterState>({
            backend: "local",
            connected: false,
            endpoint: "",
        }),
        authAtom: atom<AuthState>({
            status: "disconnected",
        }),
        userInfoAtom: atom<UserInfo | null>(null),
        providerConfigAtom: atom<ProviderConfig | null>(null),
        sessionIdAtom: atom<string>(""),
        rawOutputAtom: atom<string>(""),
    };
}

/**
 * Factory function: Create filtered document atom (derived)
 *
 * Applies filter state to document nodes in real-time.
 *
 * @param documentAtom - Instance's document atom
 * @param documentStateAtom - Instance's document state atom
 * @returns Derived atom with filtered nodes
 */
export function createFilteredDocumentAtom(
    documentAtom: PrimitiveAtom<DocumentNode[]>,
    documentStateAtom: PrimitiveAtom<DocumentState>
): Atom<DocumentNode[]> {
    return atom((get) => {
        const document = get(documentAtom);
        const state = get(documentStateAtom);

        return document.filter((node) => {
            // Filter thinking blocks
            if (node.type === "markdown" && node.metadata?.thinking && !state.filter.showThinking) {
                return false;
            }

            // Filter successful tools
            if (
                node.type === "tool" &&
                node.status === "success" &&
                !state.filter.showSuccessfulTools
            ) {
                return false;
            }

            // Filter failed tools
            if (node.type === "tool" && node.status === "failed" && !state.filter.showFailedTools) {
                return false;
            }

            // Filter incoming agent messages
            if (
                node.type === "agent_message" &&
                node.direction === "incoming" &&
                !state.filter.showIncoming
            ) {
                return false;
            }

            // Filter outgoing agent messages
            if (
                node.type === "agent_message" &&
                node.direction === "outgoing" &&
                !state.filter.showOutgoing
            ) {
                return false;
            }

            return true;
        });
    });
}

/**
 * Factory function: Create document statistics atom (derived)
 *
 * @param documentAtom - Instance's document atom
 * @returns Derived atom with document statistics
 */
export function createDocumentStatsAtom(
    documentAtom: PrimitiveAtom<DocumentNode[]>
): Atom<{
    totalNodes: number;
    markdownNodes: number;
    toolNodes: number;
    successfulTools: number;
    failedTools: number;
    runningTools: number;
    agentMessages: number;
    userMessages: number;
}> {
    return atom((get) => {
        const document = get(documentAtom);

        const stats = {
            totalNodes: document.length,
            markdownNodes: 0,
            toolNodes: 0,
            successfulTools: 0,
            failedTools: 0,
            runningTools: 0,
            agentMessages: 0,
            userMessages: 0,
        };

        document.forEach((node) => {
            switch (node.type) {
                case "markdown":
                    stats.markdownNodes++;
                    break;
                case "tool":
                    stats.toolNodes++;
                    if (node.status === "success") stats.successfulTools++;
                    if (node.status === "failed") stats.failedTools++;
                    if (node.status === "running") stats.runningTools++;
                    break;
                case "agent_message":
                    stats.agentMessages++;
                    break;
                case "user_message":
                    stats.userMessages++;
                    break;
            }
        });

        return stats;
    });
}

/**
 * Factory function: Create toggle node collapsed action
 *
 * @param documentStateAtom - Instance's document state atom
 * @returns Writable atom for toggling node collapsed state
 */
export function createToggleNodeCollapsed(
    documentStateAtom: PrimitiveAtom<DocumentState>
): WritableAtom<null, [string], void> {
    return atom(null, (get, set, nodeId: string) => {
        const state = get(documentStateAtom);
        const collapsed = new Set(state.collapsedNodes);

        if (collapsed.has(nodeId)) {
            collapsed.delete(nodeId);
        } else {
            collapsed.add(nodeId);
        }

        set(documentStateAtom, { ...state, collapsedNodes: collapsed });
    });
}

/**
 * Factory function: Create expand all nodes action
 *
 * @param documentStateAtom - Instance's document state atom
 * @returns Writable atom for expanding all nodes
 */
export function createExpandAllNodes(
    documentStateAtom: PrimitiveAtom<DocumentState>
): WritableAtom<null, [], void> {
    return atom(null, (get, set) => {
        const state = get(documentStateAtom);
        set(documentStateAtom, { ...state, collapsedNodes: new Set() });
    });
}

/**
 * Factory function: Create collapse all nodes action
 *
 * @param documentAtom - Instance's document atom
 * @param documentStateAtom - Instance's document state atom
 * @returns Writable atom for collapsing all nodes
 */
export function createCollapseAllNodes(
    documentAtom: PrimitiveAtom<DocumentNode[]>,
    documentStateAtom: PrimitiveAtom<DocumentState>
): WritableAtom<null, [], void> {
    return atom(null, (get, set) => {
        const document = get(documentAtom);
        const allNodeIds = document.map((node) => node.id);

        const state = get(documentStateAtom);
        set(documentStateAtom, { ...state, collapsedNodes: new Set(allNodeIds) });
    });
}

/**
 * Factory function: Create append document node action
 *
 * @param documentAtom - Instance's document atom
 * @returns Writable atom for appending nodes
 */
export function createAppendDocumentNode(
    documentAtom: PrimitiveAtom<DocumentNode[]>
): WritableAtom<null, [DocumentNode], void> {
    return atom(null, (get, set, node: DocumentNode) => {
        const document = get(documentAtom);
        set(documentAtom, [...document, node]);
    });
}

/**
 * Factory function: Create update document node action
 *
 * @param documentAtom - Instance's document atom
 * @returns Writable atom for updating nodes by ID
 */
export function createUpdateDocumentNode(
    documentAtom: PrimitiveAtom<DocumentNode[]>
): WritableAtom<null, [DocumentNode], void> {
    return atom(null, (get, set, updatedNode: DocumentNode) => {
        const document = get(documentAtom);
        const index = document.findIndex((node) => node.id === updatedNode.id);

        if (index !== -1) {
            const newDocument = [...document];
            newDocument[index] = updatedNode;
            set(documentAtom, newDocument);
        }
    });
}

/**
 * Factory function: Create clear document action
 *
 * @param documentAtom - Instance's document atom
 * @param documentStateAtom - Instance's document state atom
 * @returns Writable atom for clearing document
 */
export function createClearDocument(
    documentAtom: PrimitiveAtom<DocumentNode[]>,
    documentStateAtom: PrimitiveAtom<DocumentState>
): WritableAtom<null, [], void> {
    return atom(null, (get, set) => {
        set(documentAtom, []);
        set(documentStateAtom, {
            collapsedNodes: new Set(),
            scrollPosition: 0,
            selectedNode: null,
            filter: {
                showThinking: false,
                showSuccessfulTools: true,
                showFailedTools: true,
                showIncoming: true,
                showOutgoing: true,
            },
        });
    });
}

/**
 * Factory function: Create update filter action
 *
 * @param documentStateAtom - Instance's document state atom
 * @returns Writable atom for updating filter state
 */
export function createUpdateFilter(
    documentStateAtom: PrimitiveAtom<DocumentState>
): WritableAtom<null, [Partial<DocumentState["filter"]>], void> {
    return atom(null, (get, set, filterUpdates: Partial<DocumentState["filter"]>) => {
        const state = get(documentStateAtom);
        set(documentStateAtom, {
            ...state,
            filter: { ...state.filter, ...filterUpdates },
        });
    });
}
