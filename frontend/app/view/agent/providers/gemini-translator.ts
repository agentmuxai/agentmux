// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { SessionStats, StreamEvent, ToolCallEvent, ToolResultEvent } from "../types";
import type { OutputTranslator } from "./translator";

/**
 * Translates Gemini CLI `--output-format stream-json` events into StreamEvent format.
 *
 * Gemini emits NDJSON with these event types:
 *   {"type":"init","session_id":"...","model":"..."}
 *   {"type":"message","role":"user","content":"..."}
 *   {"type":"message","role":"assistant","content":"chunk","delta":true}  // streamed text chunks
 *   {"type":"tool_use","tool_name":"...","tool_id":"...","parameters":{...}}
 *   {"type":"tool_result","tool_id":"...","status":"success"|"error","output":"..."}
 *   {"type":"result","status":"success","stats":{...}}
 *
 * Notes:
 *   - assistant delta messages each carry an incremental chunk (not accumulated text).
 *     The stream-parser handles accumulation via consecutive TextEvents with the same node.
 *   - tool_use and tool_result arrive as discrete events (not streaming).
 */
export class GeminiTranslator implements OutputTranslator {
    // Map tool_id → tool_name so tool_result can report the right tool name
    private toolNameById: Map<string, string> = new Map();

    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];

        const type: string = rawEvent.type ?? "";

        switch (type) {
            case "init":
                // Lifecycle event — no display content
                return [];

            case "result": {
                // Gemini's turn-end → the provider-agnostic `session_end` the
                // conversation reducer uses to finalize the turn (which stops the
                // working spinner). Mirrors the claude/codex translators.
                const out: StreamEvent[] = [];
                // A non-"success" result surfaces its error first, like the codex
                // turn.failed case — otherwise it stops the spinner silently and
                // reads as a success.
                if (rawEvent.status && rawEvent.status !== "success") {
                    const msg: string =
                        rawEvent.error?.message ?? rawEvent.error ?? rawEvent.message ?? `gemini turn ${rawEvent.status}`;
                    out.push({ type: "text", content: `**Error:** ${msg}` });
                }
                const stats: SessionStats = {};
                const s = rawEvent.stats;
                if (s && typeof s === "object") {
                    if (typeof s.input_tokens === "number") stats.input_tokens = s.input_tokens;
                    if (typeof s.output_tokens === "number") stats.output_tokens = s.output_tokens;
                }
                out.push({ type: "session_end", stats });
                return out;
            }

            case "message": {
                if (rawEvent.role !== "assistant") return [];
                const content: string = rawEvent.content ?? "";
                if (!content) return [];
                // Each delta is an incremental chunk; the stream-parser accumulates them
                return [{ type: "text", content }];
            }

            case "tool_use": {
                const toolName: string = rawEvent.tool_name ?? "unknown";
                const toolId: string = rawEvent.tool_id ?? `tool-${Date.now()}`;
                const params: Record<string, any> = rawEvent.parameters ?? {};
                this.toolNameById.set(toolId, toolName);
                const ev: ToolCallEvent = {
                    type: "tool_call",
                    tool: toolName,
                    id: toolId,
                    params,
                };
                return [ev];
            }

            case "tool_result": {
                const toolId: string = rawEvent.tool_id ?? "";
                const toolName = this.toolNameById.get(toolId) ?? "unknown";
                const status: "success" | "failed" = rawEvent.status === "success" ? "success" : "failed";
                const output = rawEvent.output ?? "";
                const ev: ToolResultEvent = {
                    type: "tool_result",
                    tool: toolName,
                    id: toolId,
                    status,
                    result: typeof output === "string" ? { output } : output,
                };
                return [ev];
            }

            default:
                return [];
        }
    }

    reset(): void {
        this.toolNameById.clear();
    }
}
