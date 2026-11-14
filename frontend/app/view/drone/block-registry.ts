// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Block kind metadata — drives the NodeTypeBar chips, inline NodeFields,
// and per-kind default data. One source of truth shared by all node
// components and the validators.

import type { BlockKind } from "./drone-types";

interface BlockHandleSpec {
    /** xyflow handle id; "in" / "out" by convention. */
    id: string;
    label: string;
    type: "any" | "string" | "number" | "boolean" | "object";
}

export interface BlockKindMeta {
    kind: BlockKind;
    /** Unicode emoji shown on the top-bar chip + the node header. */
    emoji: string;
    label: string;
    description: string;
    /** Hex color for the node header strip. */
    color: string;
    /** Default per-kind data fields the node ships with. */
    defaultData: Record<string, unknown>;
    /** xyflow handle definitions for the canvas. */
    inputs: BlockHandleSpec[];
    outputs: BlockHandleSpec[];
}

const BLOCK_REGISTRY: Record<BlockKind, BlockKindMeta> = {
    variables: {
        kind: "variables",
        emoji: "🔢",
        label: "Variables",
        description: "Declare drone-scope variables. Read via {{var.name}}; any {{reference}} takes filters: {{a.log | tail:20 | lower}} (tail, head, json:path, lower, trunc, default).",
        color: "#a855f7",
        defaultData: {
            entries: [{ name: "example", value: "hello" }],
        },
        inputs: [],
        outputs: [{ id: "out", label: "out", type: "object" }],
    },
    agent: {
        kind: "agent",
        emoji: "🤖",
        label: "Agent",
        description: "Run an agent with a per-call task prompt.",
        color: "#3b82f6",
        defaultData: {
            // Phase 1.5: forge_agent_id was replaced by AgentRef (#835).
            // Empty strings = no account, no memory (vanilla CLI).
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
    message: {
        kind: "message",
        emoji: "✉️",
        label: "Message",
        description: "Send an agent a note. Every {{...}} value is put in as quoted data.",
        color: "#06b6d4", // colour: fixed (the block's identity colour)
        defaultData: {
            to: "",
            body: "",
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [{ id: "out", label: "out", type: "object" }],
    },
    task: {
        kind: "task",
        emoji: "📋",
        label: "Task",
        description: "File a Kanban task. Every {{...}} value is put in as quoted data.",
        color: "#f59e0b", // colour: fixed (the block's identity colour)
        defaultData: {
            title: "",
            body: "",
            target: "",
            dedupe: "",
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [{ id: "out", label: "out", type: "object" }],
    },
    notify: {
        kind: "notify",
        emoji: "🔔",
        label: "Notify",
        description: "Tell the operator, as a notification. The title is fixed; every {{...}} value in the text is put in as quoted data.",
        color: "#ec4899", // colour: fixed (the block's identity colour)
        defaultData: {
            title: "",
            body: "",
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [{ id: "out", label: "out", type: "object" }],
    },
    app_query: {
        kind: "app_query",
        emoji: "🔎",
        label: "App query",
        description: "Read AgentMux's own state: MuxBus, one agent, Kanban tasks, LAN peers or recent job runs. Read-only.",
        color: "#14b8a6", // colour: fixed (the block's identity colour)
        defaultData: {
            query: "muxbus",
            agent: "",
            holder: "",
            state: "",
            pr: "",
            job: "",
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [{ id: "out", label: "out", type: "object" }],
    },
    spawn: {
        kind: "spawn",
        emoji: "🚀",
        label: "Spawn",
        description: "Start a worker of an agent with a brief. The leader is fixed; every {{...}} value in the brief is put in as quoted data. The person is told first unless their settings allow it.",
        color: "#8b5cf6", // colour: fixed (the block's identity colour)
        defaultData: {
            leader: "",
            brief: "",
            model: "",
            account: "",
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [{ id: "out", label: "out", type: "object" }],
    },
    api: {
        kind: "api",
        emoji: "🌐",
        label: "API",
        description: "Make an HTTP request. Headers and body support {{...}}.",
        color: "#10b981",
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
        emoji: "🔀",
        label: "Condition",
        description: "Boolean expression: a == b, or combine with && || ! ( ) and contains, matches, in_set, exists. Output `result` is true / false.",
        color: "#eab308",
        defaultData: {
            expr: "",
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [
            { id: "true", label: "true", type: "any" },
            { id: "false", label: "false", type: "any" },
        ],
    },
    transform: {
        kind: "transform",
        emoji: "🧮",
        label: "Transform",
        description: "Shape a list or object with a chain of operations (map, filter, pick, sort, group, count, sum...). No code runs.",
        color: "#14b8a6", // colour: fixed (the block's identity colour)
        defaultData: {
            input: "",
            ops: [],
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [{ id: "out", label: "out", type: "object" }],
    },
    foreach: {
        kind: "foreach",
        emoji: "🔁",
        label: "For each",
        description: "Run one block once per item of a list. Reads {{item}} and {{index}}; results come out as a list.",
        color: "#6366f1", // colour: fixed (the block's identity colour)
        defaultData: {
            items: "",
            body: "",
            max_items: 100,
            on_item_error: "stop",
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [{ id: "out", label: "out", type: "object" }],
    },
    response: {
        kind: "response",
        emoji: "🏁",
        label: "Response",
        description: "Terminal output. Exactly one per drone.",
        color: "#ef4444",
        defaultData: {
            template: "",
        },
        inputs: [{ id: "in", label: "in", type: "any" }],
        outputs: [],
    },
};

/** Stable list for the NodeTypeBar — chip order is rendering order. */
export const BLOCK_KINDS: BlockKind[] = [
    "variables",
    "agent",
    "message",
    "task",
    "notify",
    "app_query",
    "spawn",
    "api",
    "condition",
    "response",
];

export function blockMeta(kind: BlockKind): BlockKindMeta {
    return BLOCK_REGISTRY[kind];
}
