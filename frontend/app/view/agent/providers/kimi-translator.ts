// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { StreamEvent, ToolCallEvent, ToolResultEvent } from "../types";
import type { OutputTranslator } from "./translator";

/**
 * Translates Kimi Code CLI `--output-format stream-json` events into StreamEvent format.
 *
 * Kimi emits NDJSON with these message types:
 *   {"role":"assistant","content":[...],"tool_calls":[...]}
 *   {"role":"tool","tool_call_id":"...","content":[...]}
 *
 * Content parts can be:
 *   - {"type":"text","text":"..."}
 *   - {"type":"think","think":"..."}
 *   - {"type":"image_url",...} (future)
 *
 * Tool calls use OpenAI-style function calling:
 *   {"type":"function","id":"tc_1","function":{"name":"Shell","arguments":"{...}"}}
 */
export class KimiTranslator implements OutputTranslator {
    private toolNameById: Map<string, string> = new Map();

    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];

        const role: string = rawEvent.role ?? "";

        switch (role) {
            case "assistant": {
                const events: StreamEvent[] = [];

                // Handle content parts (text, think)
                const content = rawEvent.content;
                if (Array.isArray(content)) {
                    for (const part of content) {
                        if (part.type === "text" && part.text) {
                            events.push({ type: "text", content: part.text });
                        } else if (part.type === "think" && part.think) {
                            events.push({ type: "thinking", content: part.think });
                        }
                    }
                } else if (typeof content === "string" && content) {
                    events.push({ type: "text", content });
                }

                // Handle tool_calls
                const toolCalls = rawEvent.tool_calls;
                if (Array.isArray(toolCalls)) {
                    for (const tc of toolCalls) {
                        if (tc.type === "function") {
                            const toolName = tc.function?.name ?? "unknown";
                            const toolId = tc.id ?? `tool-${Date.now()}`;
                            let params: Record<string, any> = {};
                            try {
                                const args = tc.function?.arguments;
                                if (typeof args === "string") {
                                    params = JSON.parse(args);
                                } else if (typeof args === "object" && args !== null) {
                                    params = args;
                                }
                            } catch {
                                params = {};
                            }
                            this.toolNameById.set(toolId, toolName);
                            events.push({ type: "tool_call", tool: toolName, id: toolId, params });
                        }
                    }
                }

                return events;
            }

            case "tool": {
                const toolId: string = rawEvent.tool_call_id ?? "";
                const toolName = this.toolNameById.get(toolId) ?? "unknown";
                const content = rawEvent.content;
                let resultText = "";
                if (Array.isArray(content)) {
                    resultText = content
                        .filter((p: any) => p.type === "text")
                        .map((p: any) => p.text)
                        .join("");
                } else if (typeof content === "string") {
                    resultText = content;
                }
                return [{
                    type: "tool_result",
                    tool: toolName,
                    id: toolId,
                    status: "success",
                    result: { content: resultText },
                }];
            }

            default:
                return [];
        }
    }

    reset(): void {
        this.toolNameById.clear();
    }
}
