// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Block kind metadata — drives the BlockPalette, default block data,
// and the InspectorPanel field schema. One source of truth shared by
// all node components and the validators.

import type { BlockKind } from "./workflows-types";

export interface BlockHandleSpec {
    /** xyflow handle id; "in" / "out" by convention. */
    id: string;
    label: string;
    type: "any" | "string" | "number" | "boolean" | "object";
}

export interface BlockKindMeta {
    kind: BlockKind;
    label: string;
    description: string;
    /** Hex color for the node header strip. */
    color: string;
    /** FontAwesome-class icon string (used by the canvas + palette). */
    icon: string;
    /** Default per-kind data fields the node ships with. */
    defaultData: Record<string, unknown>;
    /** xyflow handle definitions for the canvas. */
    inputs: BlockHandleSpec[];
    outputs: BlockHandleSpec[];
}

export const BLOCK_REGISTRY: Record<BlockKind, BlockKindMeta> = {
    variables: {
        kind: "variables",
        label: "Variables",
        description: "Declare workflow-scope variables. Read via {{var.name}}.",
        color: "#a855f7",
        icon: "font-awesome-flag",
        defaultData: {
            entries: [{ name: "example", value: "hello" }],
        },
        inputs: [],
        outputs: [{ id: "out", label: "out", type: "object" }],
    },
    agent: {
        kind: "agent",
        label: "Agent",
        description: "Run an agent with a per-call task prompt.",
        color: "#3b82f6",
        icon: "sparkles",
        defaultData: {
            // Phase 1.5: forge_agent_id was replaced by AgentRef (#835).
            // Empty strings = blank singletons (ambient creds, vanilla CLI).
            agent_ref: {
                identityId: "",
                memoryId: "",
                instanceName: "",
                workingDirectory: "",
            },
            task: "",
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [{ id: "out", label: "out", type: "object" }],
    },
    api: {
        kind: "api",
        label: "API",
        description: "Make an HTTP request. Headers and body support {{...}}.",
        color: "#10b981",
        icon: "globe",
        defaultData: {
            method: "GET",
            url: "",
            headers: {},
            body: "",
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [{ id: "out", label: "out", type: "object" }],
    },
    condition: {
        kind: "condition",
        label: "Condition",
        description: "Boolean expression. Output `result` is true / false.",
        color: "#eab308",
        icon: "code-branch",
        defaultData: {
            expr: "",
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [
            { id: "true", label: "true", type: "any" },
            { id: "false", label: "false", type: "any" },
        ],
    },
    response: {
        kind: "response",
        label: "Response",
        description: "Terminal output. Exactly one per workflow.",
        color: "#ef4444",
        icon: "flag-checkered",
        defaultData: {
            template: "",
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [],
    },
};

/** Stable list for the BlockPalette — palette order is rendering order. */
export const BLOCK_KINDS: BlockKind[] = [
    "variables",
    "agent",
    "api",
    "condition",
    "response",
];

export function blockMeta(kind: BlockKind): BlockKindMeta {
    return BLOCK_REGISTRY[kind];
}
