// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { SessionStats, StreamEvent, ToolCallEvent, ToolResultEvent } from "../types";
import type { OutputTranslator } from "./translator";

/**
 * Translates Codex CLI `exec --json` NDJSON events into StreamEvent format.
 *
 * Codex (OpenAI Codex CLI) emits these event types when run as:
 *   codex exec --json --dangerously-bypass-approvals-and-sandbox -
 *
 * Observed events:
 *   {"type":"thread.started","thread_id":"..."}
 *   {"type":"turn.started"}
 *   {"type":"item.completed","item":{"id":"...","type":"message","role":"assistant",
 *       "content":[{"type":"output_text","text":"..."}]}}
 *   {"type":"item.completed","item":{"id":"...","type":"function_call",
 *       "name":"...","arguments":"..."}}
 *   {"type":"item.completed","item":{"id":"...","type":"function_call_output",
 *       "call_id":"...","output":"..."}}
 *   {"type":"item.completed","item":{"id":"...","type":"reasoning",
 *       "content":[{"type":"thinking","thinking":"..."}]}}
 *   {"type":"turn.completed","total_usage":{...}}
 *   {"type":"error","message":"..."}
 *
 * The item.completed events are complete (not streaming deltas) — each carries the full content.
 */
export class CodexTranslator implements OutputTranslator {
    // Map call_id → function name so function_call_output can report the right tool
    private toolNameByCallId: Map<string, string> = new Map();

    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];

        const type: string = rawEvent.type ?? "";

        switch (type) {
            case "thread.started":
            case "turn.started":
                // Lifecycle events — no display content
                return [];

            case "turn.completed":
            case "turn.failed": {
                // Map codex's turn boundary to the provider-agnostic `session_end`
                // the conversation reducer uses to finalize the turn — leaving the
                // Streaming phase (which stops the working spinner). Mirrors the
                // Claude translator's `result` → `session_end`.
                const out: StreamEvent[] = [];
                // A failed turn surfaces its error first, like the item / top-level
                // error cases — otherwise it stops the spinner silently and reads
                // as a success.
                if (type === "turn.failed") {
                    const msg: string = rawEvent.error?.message ?? rawEvent.message ?? "Codex turn failed";
                    out.push({ type: "text", content: `**Error:** ${msg}` });
                }
                // codex `exec --json` has carried usage under both `total_usage`
                // and `usage` across versions — accept either.
                const usage = rawEvent.total_usage ?? rawEvent.usage;
                const stats: SessionStats = {};
                if (usage && typeof usage === "object") {
                    if (typeof usage.input_tokens === "number") stats.input_tokens = usage.input_tokens;
                    if (typeof usage.output_tokens === "number") stats.output_tokens = usage.output_tokens;
                }
                out.push({ type: "session_end", stats });
                return out;
            }

            case "item.completed": {
                const item = rawEvent.item;
                if (!item || typeof item !== "object") return [];
                return this.translateItem(item);
            }

            case "error": {
                const msg: string = rawEvent.message ?? "unknown error";
                // Only surface terminal errors, not reconnect-in-progress messages
                if (msg.includes("Reconnecting...")) return [];
                return [{ type: "text", content: `**Error:** ${msg}` }];
            }

            default:
                return [];
        }
    }

    private translateItem(item: any): StreamEvent[] {
        const itemType: string = item.type ?? "";

        switch (itemType) {
            case "agent_message": {
                // Flat format observed in practice: {"type":"agent_message","text":"..."}
                const text: string = item.text ?? "";
                if (!text) return [];
                return [{ type: "text", content: text }];
            }

            case "message": {
                if (item.role !== "assistant") return [];
                const content: any[] = item.content ?? [];
                const parts: StreamEvent[] = [];
                for (const block of content) {
                    if (block.type === "output_text" && block.text) {
                        parts.push({ type: "text", content: block.text });
                    } else if (block.type === "refusal" && block.refusal) {
                        parts.push({ type: "text", content: `*Refused: ${block.refusal}*` });
                    }
                }
                return parts;
            }

            case "reasoning": {
                const content: any[] = item.content ?? [];
                const parts: StreamEvent[] = [];
                for (const block of content) {
                    if (block.type === "thinking" && block.thinking) {
                        parts.push({ type: "thinking", content: block.thinking });
                    }
                }
                return parts;
            }

            case "function_call": {
                const name: string = item.name ?? "unknown";
                const callId: string = item.call_id ?? item.id ?? `call-${Date.now()}`;
                this.toolNameByCallId.set(callId, name);
                let params: Record<string, any> = {};
                if (item.arguments) {
                    try {
                        params = JSON.parse(item.arguments);
                    } catch {
                        params = { _raw: item.arguments };
                    }
                }
                const ev: ToolCallEvent = {
                    type: "tool_call",
                    tool: name,
                    id: callId,
                    params,
                };
                return [ev];
            }

            case "function_call_output": {
                const callId: string = item.call_id ?? "";
                const toolName = this.toolNameByCallId.get(callId) ?? "unknown";
                const output = item.output ?? "";
                const ev: ToolResultEvent = {
                    type: "tool_result",
                    tool: toolName,
                    id: callId,
                    status: "success",
                    result: typeof output === "string" ? { output } : output,
                };
                return [ev];
            }

            case "error": {
                const msg: string = item.message ?? "unknown error";
                return [{ type: "text", content: `**Error:** ${msg}` }];
            }

            default:
                return [];
        }
    }

    reset(): void {
        this.toolNameByCallId.clear();
    }
}
