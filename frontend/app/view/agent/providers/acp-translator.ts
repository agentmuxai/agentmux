// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { StreamEvent, ToolCallEvent, ToolResultEvent } from "../types";
import type { OutputTranslator } from "./translator";

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
    // Map tool call IDs to tool names for result correlation
    private toolNameById: Map<string, string> = new Map();

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
                this.toolNameById.set(toolId, toolName);
                const ev: ToolCallEvent = {
                    type: "tool_call",
                    tool: toolName,
                    id: toolId,
                    params: toolParams,
                };
                return [ev];
            }

            case "tool_result": {
                const toolId: string = params.toolCallId ?? params.id ?? "";
                const toolName = this.toolNameById.get(toolId) ?? params.toolName ?? "unknown";
                const isError = params.isError === true || params.status === "error";
                const output = params.content ?? params.output ?? "";
                const ev: ToolResultEvent = {
                    type: "tool_result",
                    tool: toolName,
                    id: toolId,
                    status: isError ? "failed" : "success",
                    result: typeof output === "string" ? { output } : output,
                };
                return [ev];
            }

            default:
                // Discard lifecycle events (initialize, session/create results, etc.)
                return [];
        }
    }

    reset(): void {
        this.toolNameById.clear();
    }
}
