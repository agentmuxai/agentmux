// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { StreamEvent, ToolCallEvent, ToolResultEvent } from "../types";
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
            case "turn.completed":
                // Lifecycle events — no display content
                return [];

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
