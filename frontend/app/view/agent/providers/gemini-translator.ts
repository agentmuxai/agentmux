// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { StreamEvent, ToolCallEvent, ToolResultEvent } from "../types";
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
            case "result":
                // Lifecycle events — no display content
                return [];

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
