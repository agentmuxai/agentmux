// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { StreamEvent } from "../types";
import type { OutputTranslator } from "./translator";
import { ToolCorrelator, wrapOutput } from "./tool-correlation";

/**
 * Universal translator for agents speaking the Agent Client Protocol (ACP).
 *
 * ACP uses JSON-RPC 2.0 over stdio. Streaming events arrive as `session/update`
 * notifications with a `type` field indicating the event kind:
 *
 *   agent_message_chunk  — incremental text from the agent
 *   agent_thought_chunk  — reasoning/thinking content
 *   tool_call            — agent invokes a tool
 *   tool_result          — tool execution result
 *
 * This single translator handles all ACP-compatible agents (OpenClaw, Kiro,
 * Gemini --acp, and any future ACP agent) without per-agent customization.
 *
 * See: https://github.com/agentclientprotocol/agent-client-protocol
 */
export class AcpTranslator implements OutputTranslator {
    // Correlates tool call IDs to tool names for result resolution.
    private tools = new ToolCorrelator();

    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];

        // ACP session/update notifications carry params with a type field.
        // The raw event may be the full JSON-RPC envelope or just the params.
        const params = rawEvent.params ?? rawEvent;
        const type: string = params.type ?? "";

        switch (type) {
            case "agent_message_chunk": {
                const content: string = params.content ?? params.text ?? "";
                if (!content) return [];
                return [{ type: "text", content }];
            }

            case "agent_thought_chunk": {
                const content: string = params.content ?? params.text ?? "";
                if (!content) return [];
                return [{ type: "thinking", content }];
            }

            case "tool_call": {
                const toolName: string = params.toolName ?? params.name ?? "unknown";
                const toolId: string = params.toolCallId ?? params.id ?? `tool-${Date.now()}`;
                const toolParams: Record<string, any> = params.input ?? params.parameters ?? {};
                return [this.tools.call(toolName, toolId, toolParams)];
            }

            case "tool_result": {
                const toolId: string = params.toolCallId ?? params.id ?? "";
                const isError = params.isError === true || params.status === "error";
                const output = params.content ?? params.output ?? "";
                // Fallback chain `map ?? params.toolName ?? "unknown"` —
                // `a ?? (b ?? c)` is equivalent, so this preserves it.
                return [
                    this.tools.result(
                        toolId,
                        isError ? "failed" : "success",
                        wrapOutput(output),
                        params.toolName ?? "unknown",
                    ),
                ];
            }

            default:
                // Discard lifecycle events (initialize, session/create results, etc.)
                return [];
        }
    }

    reset(): void {
        this.tools.reset();
    }
}
