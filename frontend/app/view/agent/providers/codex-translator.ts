// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { SessionStats, StreamEvent } from "../types";
import { ToolCorrelator, wrapOutput } from "./tool-correlation";
import type { OutputTranslator } from "./translator";

interface ItemState {
    opened: boolean;
    completed: boolean;
    lastOutput: string;
    lastText: string;
}

/** Translate Codex `exec --json` snapshots into AgentMux stream events. */
export class CodexTranslator implements OutputTranslator {
    private tools = new ToolCorrelator();
    private items = new Map<string, ItemState>();
    private seenErrors = new Set<string>();
    private terminal = false;

    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];

        switch (rawEvent.type) {
            case "thread.started":
                return [];
            case "turn.started":
                this.resetTurn();
                return [];
            case "item.started":
            case "item.updated":
            case "item.completed":
                return this.translateItem(rawEvent.item, rawEvent.type === "item.completed");
            case "turn.completed":
                return this.finishTurn(rawEvent, false);
            case "turn.failed":
                return this.finishTurn(rawEvent, true);
            case "error": {
                const error = this.parseError(rawEvent);
                if (error.message.includes("Reconnecting...")) return [];
                return this.emitError(error.code, error.message);
            }
            default:
                return [];
        }
    }

    private finishTurn(rawEvent: any, failed: boolean): StreamEvent[] {
        if (this.terminal) return [];
        this.terminal = true;

        const events: StreamEvent[] = [];
        if (failed) {
            const error = this.parseError(rawEvent.error ?? rawEvent, "Codex turn failed");
            events.push(...this.emitError(error.code, error.message));
            for (const [itemId, state] of this.items) {
                if (state.opened && !state.completed) {
                    events.push(this.tools.result(itemId, "failed", { error: error.message }));
                    state.completed = true;
                }
            }
        }

        const usage = rawEvent.total_usage ?? rawEvent.usage;
        const stats: SessionStats = {};
        if (usage && typeof usage === "object") {
            if (typeof usage.input_tokens === "number") stats.input_tokens = usage.input_tokens;
            if (typeof usage.output_tokens === "number") stats.output_tokens = usage.output_tokens;
        }
        events.push({ type: "session_end", stats });
        return events;
    }

    private translateItem(item: any, completed: boolean): StreamEvent[] {
        if (!item || typeof item !== "object") return [];
        const itemType = typeof item.type === "string" ? item.type : "";
        const itemId = typeof item.id === "string" ? item.id : `${itemType}:anonymous`;
        const state = this.itemState(itemId);
        if (state.completed) return [];

        switch (itemType) {
            case "agent_message":
                return this.translateTextSnapshot(state, item.text, "text", completed);
            case "message":
                if (item.role !== "assistant") return [];
                return this.translateTextSnapshot(state, this.messageText(item), "text", completed);
            case "reasoning":
                return this.translateTextSnapshot(state, this.reasoningText(item), "thinking", completed);
            case "command_execution":
                return this.translateCommand(itemId, state, item, completed);
            case "file_change":
                return this.translateSimpleTool(
                    itemId,
                    state,
                    "FileChange",
                    { changes: item.changes ?? [] },
                    item,
                    completed
                );
            case "mcp_tool_call": {
                const server = typeof item.server === "string" ? item.server : "unknown";
                const tool = typeof item.tool === "string" ? item.tool : "unknown";
                return this.translateSimpleTool(
                    itemId,
                    state,
                    `mcp__${server}__${tool}`,
                    this.objectValue(item.arguments),
                    item,
                    completed
                );
            }
            case "web_search":
                return this.translateSimpleTool(
                    itemId,
                    state,
                    "WebSearch",
                    { query: item.query ?? "" },
                    item,
                    completed
                );
            case "plan_update":
                return this.translateTextSnapshot(
                    state,
                    typeof item.text === "string" ? item.text : JSON.stringify(item.plan ?? item),
                    "thinking",
                    completed
                );
            case "function_call":
                return this.translateLegacyFunctionCall(item, state, completed);
            case "function_call_output": {
                state.completed = completed;
                return [this.tools.result(item.call_id ?? itemId, "success", wrapOutput(item.output ?? ""))];
            }
            case "error": {
                state.completed = completed;
                const error = this.parseError(item);
                return this.emitError(error.code, error.message);
            }
            default:
                state.completed = completed;
                return [];
        }
    }

    private translateCommand(itemId: string, state: ItemState, item: any, completed: boolean): StreamEvent[] {
        const events: StreamEvent[] = [];
        if (!state.opened) {
            events.push(this.tools.call("Shell", itemId, { command: item.command ?? "" }));
            state.opened = true;
        }

        const output = typeof item.aggregated_output === "string" ? item.aggregated_output : "";
        const chunk = this.snapshotSuffix(state.lastOutput, output);
        if (chunk) events.push({ type: "tool_chunk", id: itemId, kind: "stdout", content: chunk });
        state.lastOutput = output;

        if (completed) {
            const succeeded = item.status === "completed" && (item.exit_code == null || item.exit_code === 0);
            const result = this.tools.result(itemId, succeeded ? "success" : "failed", {
                output,
                status: item.status,
            });
            if (typeof item.exit_code === "number") result.exitCode = item.exit_code;
            events.push(result);
            state.completed = true;
        }
        return events;
    }

    private translateSimpleTool(
        itemId: string,
        state: ItemState,
        name: string,
        params: Record<string, any>,
        item: any,
        completed: boolean
    ): StreamEvent[] {
        const events: StreamEvent[] = [];
        if (!state.opened) {
            events.push(this.tools.call(name, itemId, params));
            state.opened = true;
        }
        if (completed) {
            const failed = item.status === "failed" || item.status === "error" || item.error != null;
            const result = item.result ?? item.error ?? item.changes ?? item;
            events.push(this.tools.result(itemId, failed ? "failed" : "success", result, name));
            state.completed = true;
        }
        return events;
    }

    private translateLegacyFunctionCall(item: any, state: ItemState, completed: boolean): StreamEvent[] {
        if (state.opened) return [];
        const name = typeof item.name === "string" ? item.name : "unknown";
        const callId = item.call_id ?? item.id ?? `call-${Date.now()}`;
        state.opened = true;
        state.completed = completed;
        return [this.tools.call(name, callId, this.objectValue(item.arguments))];
    }

    private translateTextSnapshot(
        state: ItemState,
        value: unknown,
        type: "text" | "thinking",
        completed: boolean
    ): StreamEvent[] {
        const text = typeof value === "string" ? value : "";
        const suffix = this.snapshotSuffix(state.lastText, text);
        state.lastText = text;
        state.completed = completed;
        return suffix ? [{ type, content: suffix }] : [];
    }

    private messageText(item: any): string {
        if (!Array.isArray(item.content)) return "";
        return item.content
            .map((block: any) => {
                if (block?.type === "output_text") return block.text ?? "";
                if (block?.type === "refusal") return block.refusal ? `Refused: ${block.refusal}` : "";
                return "";
            })
            .join("");
    }

    private reasoningText(item: any): string {
        if (typeof item.text === "string") return item.text;
        if (!Array.isArray(item.content)) return "";
        return item.content.map((block: any) => block?.thinking ?? block?.text ?? "").join("");
    }

    private objectValue(value: unknown): Record<string, any> {
        if (value && typeof value === "object" && !Array.isArray(value)) return value as Record<string, any>;
        if (typeof value === "string" && value) {
            try {
                const parsed = JSON.parse(value);
                if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) return parsed;
            } catch {
                return { _raw: value };
            }
        }
        return {};
    }

    private snapshotSuffix(previous: string, next: string): string {
        if (!next || next === previous) return "";
        return next.startsWith(previous) ? next.slice(previous.length) : next;
    }

    private parseError(value: any, fallback = "unknown error"): { code: number; message: string } {
        let code = typeof value?.status === "number" ? value.status : 0;
        let message = value?.error?.message ?? value?.message ?? fallback;
        if (typeof message !== "string") message = String(message ?? fallback);

        try {
            const nested = JSON.parse(message);
            if (typeof nested?.status === "number") code = nested.status;
            if (typeof nested?.error?.message === "string") message = nested.error.message;
            else if (typeof nested?.message === "string") message = nested.message;
        } catch {
            // Provider error messages are often plain strings.
        }
        return { code, message };
    }

    private emitError(code: number, message: string): StreamEvent[] {
        const key = `${code}:${message}`;
        if (this.seenErrors.has(key)) return [];
        this.seenErrors.add(key);
        return [{ type: "error_result", code, message }];
    }

    private itemState(id: string): ItemState {
        const existing = this.items.get(id);
        if (existing) return existing;
        const state: ItemState = { opened: false, completed: false, lastOutput: "", lastText: "" };
        this.items.set(id, state);
        return state;
    }

    private resetTurn(): void {
        this.items.clear();
        this.seenErrors.clear();
        this.terminal = false;
    }

    reset(): void {
        this.tools.reset();
        this.resetTurn();
    }
}
