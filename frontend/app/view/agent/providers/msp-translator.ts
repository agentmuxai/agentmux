// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { SessionStats, StreamEvent } from "../types";
import { ToolCorrelator, wrapOutput } from "./tool-correlation";
import type { OutputTranslator } from "./translator";

/**
 * Translator for Muse Code's own protocol, MSP (`muse serve`): JSON-RPC 2.0
 * over stdio, which srv's MspController writes to the transcript line by line
 * (crates/srv/src/backend/blockcontroller/msp.rs). Checked against Muse Code
 * 1.4.7-R6101.1's schema (`muse schema generate-ts`).
 *
 * The transcript is items, each with a kind and a lifecycle:
 *
 *   item/started    — an item opened, the full object at revision 1
 *   item/delta      — text appended to one of its fields (`field`, default
 *                     "text"): an `agentMessage`'s reply, a `reasoning`
 *                     item's `summary.N`, a tool's `output`
 *   item/updated    — the full object again, at a higher revision
 *   item/completed  — the final object; may come without a start
 *
 * and turns: `turn/completed` ends one, with `error` when it failed
 * (`authRequired` when Muse isn't signed in). `session/tokenUsage` reports
 * each model call's usage. Everything else (session and status traffic,
 * Muse's internal `reminderChild` items, command acks) is not rendered.
 */
export class MspTranslator implements OutputTranslator {
    private tools = new ToolCorrelator();
    /** Open items by id: their kind and what has been shown of them. */
    private items = new Map<string, ItemState>();
    /** Tool items already ended, so a repeated completion doesn't end one twice. */
    private ended = new Set<string>();
    /** The turn's usage so far, summed over its model calls. */
    private usage = { prompt: 0, output: 0, seen: false };

    constructor(private readonly opts: { replay?: boolean } = {}) {}

    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];
        const method = rawEvent.method;
        if (typeof method !== "string") {
            // A response: only an error is shown (a refused prompt, a session
            // Muse couldn't open, or Muse exiting, which srv reports as one).
            return errorEvents(rawEvent.error);
        }
        const params = rawEvent.params ?? {};
        switch (method) {
            case "item/started":
            case "item/updated":
                return this.onItem(params.item, false);
            case "item/completed":
                return this.onItem(params.item, true);
            case "item/delta":
                return this.onDelta(params);
            case "session/tokenUsage":
                this.onUsage(params);
                return [];
            case "turn/completed":
                return this.onTurnCompleted(params);
            default:
                return [];
        }
    }

    private onItem(item: any, completed: boolean): StreamEvent[] {
        if (!item || typeof item !== "object" || typeof item.itemId !== "string") return [];
        const id: string = item.itemId;
        const kind: string = typeof item.kind === "string" ? item.kind : "";
        let state = this.items.get(id);
        if (!state) {
            state = { kind, text: "", summary: [] };
            this.items.set(id, state);
        }
        const events: StreamEvent[] = [];
        switch (kind) {
            case "agentMessage":
                events.push(...catchUp(state, stringOr(item.text)));
                break;
            case "reasoning": {
                // Shown as streamed; a reasoning item that arrives whole is
                // shown at completion.
                if (completed) {
                    const parts: string[] = Array.isArray(item.summary) ? item.summary.filter((s: any) => typeof s === "string") : [];
                    const whole = parts.length > 0 ? parts.join("\n\n") : stringOr(item.text);
                    const shown = state.summary.join("\n\n");
                    if (!shown && whole) events.push({ type: "thinking", content: whole });
                }
                break;
            }
            case "userMessage":
                // Live, the pane already shows the message it sent; from
                // history it is the user's turn.
                if (completed && this.opts.replay) {
                    const message = stringOr(item.displayText) || stringOr(item.text);
                    if (message) events.push({ type: "user_message", message });
                }
                break;
            default: {
                const tool = toolOf(item);
                if (!tool) break;
                if (!this.ended.has(id) && (!state.called || state.args !== tool.argsKey)) {
                    state.called = true;
                    state.args = tool.argsKey;
                    events.push(this.tools.call(tool.name, id, tool.params));
                }
                if (completed && !this.ended.has(id)) {
                    this.ended.add(id);
                    events.push(this.tools.result(id, toolStatus(item.status), wrapOutput(toolOutput(item)), tool.name));
                }
            }
        }
        if (completed) this.items.delete(id);
        return events;
    }

    private onDelta(params: any): StreamEvent[] {
        const state = this.items.get(params.itemId);
        const delta = stringOr(params.delta);
        if (!state || !delta) return [];
        const field = typeof params.field === "string" && params.field ? params.field : "text";
        if (state.kind === "agentMessage" && field === "text") {
            state.text += delta;
            return [{ type: "text", content: delta }];
        }
        if (state.kind === "reasoning" && field.startsWith("summary.")) {
            const n = Number(field.slice("summary.".length));
            if (!Number.isInteger(n) || n < 0) return [];
            // A new part starts a new paragraph.
            const sep = state.summary[n] == null && n > 0 ? "\n\n" : "";
            state.summary[n] = (state.summary[n] ?? "") + delta;
            return [{ type: "thinking", content: sep + delta }];
        }
        return [];
    }

    private onUsage(params: any): void {
        if (typeof params.promptTokens === "number") this.usage.prompt += params.promptTokens;
        const out = params.usage?.outputTokens;
        if (typeof out === "number") this.usage.output += out;
        this.usage.seen = true;
    }

    private onTurnCompleted(params: any): StreamEvent[] {
        const events: StreamEvent[] = [];
        if (params.error && typeof params.error === "object") {
            events.push({ type: "text", content: turnErrorText(params.error) });
        }
        // Tools still open when the turn ended didn't finish.
        for (const [id, state] of this.items) {
            if (state.called && !this.ended.has(id)) {
                events.push(this.tools.result(id, "failed", { output: "" }));
            }
        }
        const stats: SessionStats = {};
        if (typeof params.durationMs === "number") stats.duration_ms = params.durationMs;
        if (this.usage.seen) {
            // `promptTokens` is counted once under the provider's cache
            // convention, per model call; the turn's figure is their sum.
            stats.input_tokens = this.usage.prompt;
            stats.output_tokens = this.usage.output;
        }
        events.push({ type: "session_end", stats });
        this.items.clear();
        this.ended.clear();
        this.usage = { prompt: 0, output: 0, seen: false };
        return events;
    }

    reset(): void {
        this.tools.reset();
        this.items.clear();
        this.ended.clear();
        this.usage = { prompt: 0, output: 0, seen: false };
    }
}

interface ItemState {
    kind: string;
    /** An agent message's text shown so far. */
    text: string;
    /** A reasoning item's summary parts shown so far. */
    summary: string[];
    /** A tool item: its call was shown, with these args. */
    called?: boolean;
    args?: string;
}

/** What an agent message's final text adds to what was streamed, if anything. */
function catchUp(state: ItemState, full: string): StreamEvent[] {
    // The item came whole, or carries more than was streamed. A final text
    // that doesn't extend the streamed one (Muse cut it at its text budget)
    // adds nothing: the stream already showed it.
    if (!full || !full.startsWith(state.text) || full === state.text) return [];
    const rest = full.slice(state.text.length);
    state.text = full;
    return [{ type: "text", content: rest }];
}

function stringOr(v: unknown): string {
    return typeof v === "string" ? v : "";
}

/** A tool-like item's display name and input, or null for any other kind. */
function toolOf(item: any): { name: string; params: Record<string, any>; argsKey: string } | null {
    switch (item.kind) {
        case "toolCall": {
            const args = stringOr(item.args);
            return { name: stringOr(item.tool) || "tool", params: parseArgs(args), argsKey: args };
        }
        case "userShell": {
            const command = stringOr(item.commandText);
            return { name: "shell", params: { command }, argsKey: command };
        }
        case "subagent": {
            const objective = stringOr(item.objective);
            return { name: "subagent", params: { role: stringOr(item.role), objective }, argsKey: objective };
        }
        case "webSearch":
            return { name: "web_search", params: {}, argsKey: "" };
        default:
            return null;
    }
}

/** A tool call's arguments: the model's JSON, verbatim; an object when it parses. */
function parseArgs(args: string): Record<string, any> {
    if (!args) return {};
    try {
        const v = JSON.parse(args);
        return v && typeof v === "object" && !Array.isArray(v) ? v : { args: v };
    } catch {
        return { args };
    }
}

function toolStatus(status: unknown): "success" | "failed" | "denied" {
    if (status === "completed") return "success";
    if (status === "rejected") return "denied";
    return "failed";
}

function toolOutput(item: any): string {
    const out = stringOr(item.visibleOutput) || stringOr(item.result?.summary) || stringOr(item.result?.text);
    if (out) return out;
    return stringOr(item.failureReason);
}

/** The pane's text for a failed turn. Signed out, it says how to sign in. */
function turnErrorText(error: any): string {
    const message = stringOr(error.message) || "the turn failed";
    if (error.kind === "authRequired") {
        return (
            "**Muse Code isn't signed in.** Type `/login` to sign in (it runs `muse login` for this agent), " +
            `or set META_API_KEY in the agent's environment, then send your message again. (Muse: ${message})`
        );
    }
    return `**Error:** ${message}`;
}

/** A JSON-RPC error object as the pane's error text; nothing for anything else. */
function errorEvents(error: any): StreamEvent[] {
    if (!error || typeof error !== "object") return [];
    const message = stringOr(error.message) || "Muse Code returned an error";
    return [{ type: "text", content: `**Error:** ${message}` }];
}
