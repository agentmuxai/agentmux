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

/** AgentMux's Bash wrapper as the CLI quotes it in a task's summary, with the
 *  agent's command base64url-encoded in `--b64-cmd` (crates/bashwrap/src/hook.rs). */
const WRAPPED_COMMAND = /agentmux-bashwrap exec(?: --tool-id=\S+)? --b64-cmd=([A-Za-z0-9_-]*)(?: --declared-background)?/;
/** Longest command shown in place of a wrapper. */
const MAX_COMMAND_CHARS = 80;
/** Bash descriptions remembered, for the tasks still to finish. */
const MAX_DESCRIPTIONS = 200;

function decodeBase64Url(b64: string): string | null {
    try {
        const std = b64.replace(/-/g, "+").replace(/_/g, "/");
        const bytes = Uint8Array.from(atob(std + "=".repeat((4 - (std.length % 4)) % 4)), (c) => c.charCodeAt(0));
        return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    } catch {
        return null;
    }
}

/**
 * A summary that quotes AgentMux's wrapper instead of what the agent ran:
 * until the hook kept the call's other fields, the CLI lost the call's
 * description and labelled the task with the wrapped command, and transcripts
 * keep that summary. Shows the call's description when it is known, else the
 * command itself (its first line, shortened).
 * docs/retro/RETRO_BASHWRAP_HOOK_DROPS_BASH_TOOL_FIELDS_2026_10_10.md.
 */
export function unwrapTaskSummary(summary: string, description?: string): string {
    const match = WRAPPED_COMMAND.exec(summary);
    if (!match) return summary;
    let shown = description?.trim();
    if (!shown) {
        const command = decodeBase64Url(match[1])?.trim().split(/\r?\n/)[0]?.trim();
        shown = command ? (command.length > MAX_COMMAND_CHARS ? `${command.slice(0, MAX_COMMAND_CHARS - 1)}…` : command) : "a command";
    }
    return summary.replace(match[0], () => shown);
}

/** Feed every stream line in order; returns the wake line when one is due. */
export type TaskWakeDetector = (line: unknown, stampMs?: number) => AmbientNarrationNode | null;

export function createTaskWakeDetector(): TaskWakeDetector {
    let pending: { id: string; summary: string | null } | null = null;
    // The description of each Bash call, by tool-use id, for a summary that
    // quotes the wrapper instead.
    const descriptions = new Map<string, string>();
    return (raw, stampMs) => {
        const line = (raw ?? {}) as StreamLine;
        if (line.parent_tool_use_id) return null;
        if (line.type === "assistant") {
            rememberBashDescriptions(line, descriptions);
            return null;
        }
        if (line.type === "system" && line.subtype === "task_notification") {
            const id = typeof line.task_id === "string" && line.task_id ? line.task_id : typeof line.tool_use_id === "string" ? line.tool_use_id : null;
            const summary = typeof line.summary === "string" && line.summary.trim() ? line.summary.trim() : null;
            const description = typeof line.tool_use_id === "string" ? descriptions.get(line.tool_use_id) : undefined;
            if (id) pending = { id, summary: summary && unwrapTaskSummary(summary, description) };
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

function rememberBashDescriptions(line: StreamLine, descriptions: Map<string, string>): void {
    const content = line.message?.content;
    if (!Array.isArray(content)) return;
    for (const block of content as Array<{ type?: unknown; name?: unknown; id?: unknown; input?: { description?: unknown } }>) {
        if (block?.type !== "tool_use" || block.name !== "Bash" || typeof block.id !== "string") continue;
        const description = block.input?.description;
        if (typeof description !== "string" || !description.trim()) continue;
        descriptions.delete(block.id);
        descriptions.set(block.id, description.trim());
        if (descriptions.size > MAX_DESCRIPTIONS) descriptions.delete(descriptions.keys().next().value as string);
    }
}
