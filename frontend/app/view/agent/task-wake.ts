// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pass the CLI started by itself for a finished background task, found in
 * the stream: a main-agent `system/task_notification`, then a `system/init`
 * with no input written in between. The CLI echoes nothing for such a pass,
 * so without this the agent's reply arrives out of nowhere. Both the live
 * stream and history replay feed every line here in order, and the node id
 * comes from the task's own id, so the two always produce the same node and
 * the document store merges them (as compact-boundary.ts does for compaction
 * cards). The line is marked as AgentMux's, not the model's.
 *
 * docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §5.3, §5.4.
 */

import type { AmbientNarrationNode } from "./types";

interface StreamLine {
    type?: string;
    subtype?: string;
    task_id?: unknown;
    tool_use_id?: unknown;
    summary?: unknown;
    parent_tool_use_id?: unknown;
    message?: { content?: unknown };
}

/** A user line that is input (the pane's message, a jekt), not a tool's result. */
function isInput(line: StreamLine): boolean {
    if (line.type !== "user") return false;
    const content = line.message?.content;
    return !(Array.isArray(content) && content.some((b) => (b as { type?: unknown } | null)?.type === "tool_result"));
}

/** Feed every stream line in order; returns the wake line when one is due. */
export type TaskWakeDetector = (line: unknown, stampMs?: number) => AmbientNarrationNode | null;

export function createTaskWakeDetector(): TaskWakeDetector {
    let pending: { id: string; summary: string | null } | null = null;
    return (raw, stampMs) => {
        const line = (raw ?? {}) as StreamLine;
        if (line.parent_tool_use_id) return null;
        if (line.type === "system" && line.subtype === "task_notification") {
            const id = typeof line.task_id === "string" && line.task_id ? line.task_id : typeof line.tool_use_id === "string" ? line.tool_use_id : null;
            if (id) pending = { id, summary: typeof line.summary === "string" && line.summary.trim() ? line.summary.trim() : null };
            return null;
        }
        if (isInput(line)) {
            // Input started (or reached) the next pass: it is that input's.
            pending = null;
            return null;
        }
        if (line.type === "system" && line.subtype === "init" && pending) {
            const { id, summary } = pending;
            pending = null;
            return {
                type: "ambient_narration",
                id: `task-wake-${id}`,
                kind: "turn_trigger",
                text: `Woke up: ${summary ?? "a background task finished"}`,
                timestamp: stampMs ?? Date.now(),
            };
        }
        return null;
    };
}
