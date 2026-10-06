// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { StreamEvent } from "../types";
import type { OutputTranslator } from "./translator";
import { ToolCorrelator, wrapOutput } from "./tool-correlation";

/**
 * Universal translator for agents speaking the Agent Client Protocol (ACP).
 *
 * ACP is JSON-RPC 2.0 over stdio, and srv writes each line to the transcript as
 * it came (blockcontroller/acp.rs). Streaming arrives as `session/update`
 * notifications whose kind is `params.update.sessionUpdate`
 * (https://agentclientprotocol.com/protocol/prompt-turn):
 *
 *   agent_message_chunk  — `content` is a ContentBlock (`{type: "text", text}`)
 *   agent_thought_chunk  — reasoning, same shape
 *   tool_call            — `toolCallId`, `title`, `kind`, `status`, `rawInput`, …
 *   tool_call_update     — the same call later: may fill in `rawInput`, `title`
 *                          or `kind` (re-emitted as the same call, which the
 *                          parser upgrades in place); `completed` / `failed`
 *                          ends it, with `content` (text, diff, terminal)
 *                          and/or `rawOutput`
 *   plan, usage_update, available_commands_update, current_mode_update,
 *   user_message_chunk   — not rendered
 *
 * The turn ends with the `session/prompt` response's `result.stopReason`.
 *
 * The first version read a flat `params.type` (`agent_message_chunk`,
 * `tool_call`, `tool_result` with `content`/`toolName`/`input`), which no ACP
 * agent sends, so a spec-compliant agent's pane stayed empty. That shape is
 * still accepted, in case some agent emits it.
 */
export class AcpTranslator implements OutputTranslator {
    // Correlates tool call IDs to tool names for result resolution.
    private tools = new ToolCorrelator();
    // Tool calls already ended, so a repeated `completed` update (or one after
    // a `tool_call` that arrived already complete) doesn't end it twice.
    private ended = new Set<string>();
    // Each open call's name and input as known so far: an update may carry
    // the input (often empty at first) or a better title later.
    private calls = new Map<string, { name: string; input: Record<string, any> }>();

    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];

        // The `session/prompt` response: the turn is over.
        const stopReason = rawEvent.result?.stopReason;
        if (typeof stopReason === "string") return [{ type: "session_end", stats: {} }];

        // The raw event may be the full JSON-RPC envelope or just the params.
        const params = rawEvent.params ?? rawEvent;
        const update = params?.update;
        if (update && typeof update === "object" && typeof update.sessionUpdate === "string") {
            return this.translateUpdate(update);
        }
        return this.translateFlat(params);
    }

    /** A standard `session/update`. */
    private translateUpdate(u: any): StreamEvent[] {
        switch (u.sessionUpdate) {
            case "agent_message_chunk": {
                const content = blockText(u.content);
                return content ? [{ type: "text", content }] : [];
            }
            case "agent_thought_chunk": {
                const content = blockText(u.content);
                return content ? [{ type: "thinking", content }] : [];
            }
            case "tool_call": {
                const toolId = typeof u.toolCallId === "string" && u.toolCallId ? u.toolCallId : `tool-${Date.now()}`;
                const call = { name: toolName(u), input: toolInput(u) };
                this.calls.set(toolId, call);
                const events: StreamEvent[] = [this.tools.call(call.name, toolId, call.input)];
                // An agent may report a call that has already finished.
                const end = this.endIfDone(toolId, u);
                if (end) events.push(end);
                return events;
            }
            case "tool_call_update": {
                const toolId = typeof u.toolCallId === "string" ? u.toolCallId : "";
                if (!toolId) return [];
                const events: StreamEvent[] = [];
                // New input or a new name: the same call again, upgraded in place.
                const known = this.calls.get(toolId);
                if (known && !this.ended.has(toolId)) {
                    const hasName = [u.name, u.title, u.kind].some((v) => typeof v === "string" && v);
                    const name = hasName ? toolName(u) : known.name;
                    const input = Object.keys(toolInput(u)).length > 0 ? toolInput(u) : known.input;
                    // By content: each frame parses to a fresh object.
                    if (name !== known.name || JSON.stringify(input) !== JSON.stringify(known.input)) {
                        const call = { name, input };
                        this.calls.set(toolId, call);
                        events.push(this.tools.call(call.name, toolId, call.input));
                    }
                }
                const end = this.endIfDone(toolId, u);
                if (end) events.push(end);
                return events;
            }
            default:
                return [];
        }
    }

    /** The tool result for a call whose status is `completed` or `failed`. */
    private endIfDone(toolId: string, u: any): StreamEvent | null {
        if (u.status !== "completed" && u.status !== "failed") return null;
        if (this.ended.has(toolId)) return null;
        this.ended.add(toolId);
        const name = this.calls.get(toolId)?.name ?? toolName(u);
        this.calls.delete(toolId);
        return this.tools.result(toolId, u.status === "failed" ? "failed" : "success", wrapOutput(toolOutput(u)), name);
    }

    /** The flat shape the first version expected (`params.type`). */
    private translateFlat(params: any): StreamEvent[] {
        const type: string = params?.type ?? "";
        switch (type) {
            case "agent_message_chunk": {
                const content: string = params.content ?? params.text ?? "";
                return content ? [{ type: "text", content }] : [];
            }
            case "agent_thought_chunk": {
                const content: string = params.content ?? params.text ?? "";
                return content ? [{ type: "thinking", content }] : [];
            }
            case "tool_call": {
                const name: string = params.toolName ?? params.name ?? "unknown";
                const toolId: string = params.toolCallId ?? params.id ?? `tool-${Date.now()}`;
                const toolParams: Record<string, any> = params.input ?? params.parameters ?? {};
                return [this.tools.call(name, toolId, toolParams)];
            }
            case "tool_result": {
                const toolId: string = params.toolCallId ?? params.id ?? "";
                const isError = params.isError === true || params.status === "error";
                const output = params.content ?? params.output ?? "";
                return [this.tools.result(toolId, isError ? "failed" : "success", wrapOutput(output), params.toolName ?? "unknown")];
            }
            default:
                // Lifecycle traffic (initialize, session/new results, …).
                return [];
        }
    }

    reset(): void {
        this.tools.reset();
        this.ended.clear();
        this.calls.clear();
    }
}

/** The text of a ContentBlock (`{type: "text", text}`); "" for anything else. */
function blockText(block: any): string {
    if (typeof block === "string") return block;
    if (block && typeof block === "object" && block.type === "text" && typeof block.text === "string") return block.text;
    return "";
}

/** A tool call's display name: its `name`, else its `title`, else its `kind`. */
function toolName(u: any): string {
    for (const v of [u.name, u.title, u.kind]) {
        if (typeof v === "string" && v) return v;
    }
    return "tool";
}

/** A tool call's input: `rawInput` when it is an object. */
function toolInput(u: any): Record<string, any> {
    return u.rawInput && typeof u.rawInput === "object" && !Array.isArray(u.rawInput) ? u.rawInput : {};
}

/**
 * A finished tool call's output, as text: its `content` items (text blocks,
 * diffs as `path` plus the new text, terminals by id), else `rawOutput`.
 */
function toolOutput(u: any): string {
    const parts: string[] = [];
    if (Array.isArray(u.content)) {
        for (const item of u.content) {
            if (!item || typeof item !== "object") continue;
            if (item.type === "content") {
                const t = blockText(item.content);
                if (t) parts.push(t);
            } else if (item.type === "diff" && typeof item.path === "string") {
                parts.push(`${item.path}\n${typeof item.newText === "string" ? item.newText : ""}`);
            } else if (item.type === "terminal" && typeof item.terminalId === "string") {
                parts.push(`[terminal ${item.terminalId}]`);
            }
        }
    }
    if (parts.length > 0) return parts.join("\n");
    if (u.rawOutput == null) return "";
    return typeof u.rawOutput === "string" ? u.rawOutput : JSON.stringify(u.rawOutput, null, 2);
}
