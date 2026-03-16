// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { StreamEvent } from "../types";
import type { OutputTranslator } from "./translator";

/**
 * Translates Claude Code CLI stream-json output into StreamEvent format.
 *
 * Claude CLI (--output-format stream-json) wraps events in:
 *   {"type":"stream_event","event":{...}}
 *
 * The inner events use Anthropic's Messages API format:
 *   - message_start: may contain tool_result blocks (role:"user")
 *   - content_block_start: starts a new content block
 *   - content_block_delta: incremental content (text, thinking, tool_use input)
 *   - content_block_stop: ends a content block
 *   - message_delta: final message metadata (stop_reason, usage)
 *   - message_stop: end of message
 *   - result: final result with cost info
 *
 * Events that already match StreamEvent format are passed through directly.
 */
export class ClaudeTranslator implements OutputTranslator {
    private currentToolCallId: string | null = null;
    private currentToolName: string | null = null;
    private toolInputBuffer: string = "";
    private hasReceivedStreamEvents: boolean = false;

    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];

        // Case 1: Already a StreamEvent (type is text/thinking/tool_call/tool_result/etc.)
        if (this.isStreamEvent(rawEvent)) {
            return [rawEvent as StreamEvent];
        }

        // Case 2: Wrapped in {"type":"stream_event","event":{...}}
        if (rawEvent.type === "stream_event" && rawEvent.event) {
            this.hasReceivedStreamEvents = true;
            return this.translateInnerEvent(rawEvent.event);
        }

        // Case 3: Top-level "assistant" event (complete message)
        if (rawEvent.type === "assistant" && rawEvent.message) {
            return this.handleAssistantMessage(rawEvent.message);
        }

        // Case 4: Top-level "user" event (tool results)
        if (rawEvent.type === "user" && rawEvent.message) {
            return this.handleUserMessage(rawEvent.message);
        }

        // Case 5: Raw Anthropic API event (content_block_delta, etc.)
        if (this.isAnthropicEvent(rawEvent)) {
            return this.translateInnerEvent(rawEvent);
        }

        // Unknown format - discard
        return [];
    }

    reset(): void {
        this.currentToolCallId = null;
        this.currentToolName = null;
        this.toolInputBuffer = "";
        this.hasReceivedStreamEvents = false;
    }

    private isStreamEvent(event: any): boolean {
        const streamTypes = [
            "text",
            "thinking",
            "tool_call",
            "tool_result",
            "agent_message",
            "user_message",
        ];
        return streamTypes.includes(event.type);
    }

    private isAnthropicEvent(event: any): boolean {
        const anthropicTypes = [
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop",
        ];
        return anthropicTypes.includes(event.type);
    }

    private translateInnerEvent(event: any): StreamEvent[] {
        switch (event.type) {
            case "message_start":
                return this.handleMessageStart(event);

            case "content_block_start":
                return this.handleContentBlockStart(event);

            case "content_block_delta":
                return this.handleContentBlockDelta(event);

            case "content_block_stop":
                return this.handleContentBlockStop(event);

            case "message_delta":
            case "message_stop":
            case "ping":
                // Metadata events - discard
                return [];

            default:
                return [];
        }
    }

    /**
     * Top-level "assistant" event contains the complete message.
     * When --include-partial-messages is used, stream_events arrive first and
     * this duplicates them — so we skip text that we've already seen via deltas.
     * When partial messages are NOT available, this is the only source of content.
     */
    private handleAssistantMessage(message: any): StreamEvent[] {
        if (!message || !Array.isArray(message.content)) return [];

        // If we've already received stream_event content, skip to avoid duplicates
        if (this.hasReceivedStreamEvents) return [];

        // Fallback: extract content from complete message
        const results: StreamEvent[] = [];
        for (const block of message.content) {
            if (block.type === "text" && block.text) {
                results.push({ type: "text", content: block.text });
            } else if (block.type === "thinking" && block.thinking) {
                results.push({ type: "thinking", content: block.thinking });
            } else if (block.type === "tool_use") {
                results.push({
                    type: "tool_call",
                    tool: block.name || "Unknown",
                    id: block.id || `tool_${Date.now()}`,
                    params: block.input || {},
                });
            }
        }
        return results;
    }

    /**
     * Top-level "user" event contains tool_result blocks.
     */
    private handleUserMessage(message: any): StreamEvent[] {
        const content = message.content;
        if (!content) return [];

        // Handle string content
        if (typeof content === "string") {
            return [{ type: "user_message", message: content, timestamp: Date.now() }];
        }

        // Handle array content with tool_result blocks
        if (Array.isArray(content)) {
            const results: StreamEvent[] = [];
            for (const block of content) {
                if (block.type === "tool_result") {
                    const isError = block.is_error === true;
                    results.push({
                        type: "tool_result",
                        tool: block.tool_name || "Unknown",
                        id: block.tool_use_id || `tool_${Date.now()}`,
                        status: isError ? "failed" : "success",
                        result: typeof block.content === "string"
                            ? { content: block.content }
                            : block.content,
                    });
                }
            }
            return results;
        }

        return [];
    }

    /**
     * message_start may contain tool_result blocks when role is "user"
     * (these are the results of tool calls being fed back to Claude)
     */
    private handleMessageStart(event: any): StreamEvent[] {
        const message = event.message;
        if (!message) return [];

        // Check for tool_result blocks in user messages
        if (message.role === "user" && Array.isArray(message.content)) {
            const results: StreamEvent[] = [];
            for (const block of message.content) {
                if (block.type === "tool_result") {
                    const isError = block.is_error === true;
                    results.push({
                        type: "tool_result",
                        tool: block.tool_name || "Unknown",
                        id: block.tool_use_id || `tool_${Date.now()}`,
                        status: isError ? "failed" : "success",
                        result: typeof block.content === "string"
                            ? { content: block.content }
                            : block.content,
                    });
                }
            }
            return results;
        }

        return [];
    }

    /**
     * content_block_start begins a new content block.
     * For tool_use blocks, emit a tool_call event.
     */
    private handleContentBlockStart(event: any): StreamEvent[] {
        const block = event.content_block;
        if (!block) return [];

        if (block.type === "tool_use") {
            this.currentToolCallId = block.id || `tool_${Date.now()}`;
            this.currentToolName = block.name || "Unknown";
            this.toolInputBuffer = "";

            return [
                {
                    type: "tool_call",
                    tool: this.currentToolName!,
                    id: this.currentToolCallId!,
                    params: {},
                },
            ];
        }

        // text or thinking blocks start empty - wait for deltas
        return [];
    }

    /**
     * content_block_delta provides incremental content.
     */
    private handleContentBlockDelta(event: any): StreamEvent[] {
        const delta = event.delta;
        if (!delta) return [];

        switch (delta.type) {
            case "text_delta":
                if (delta.text) {
                    return [{ type: "text", content: delta.text }];
                }
                break;

            case "thinking_delta":
                if (delta.thinking) {
                    return [{ type: "thinking", content: delta.thinking }];
                }
                break;

            case "input_json_delta":
                // Accumulate tool input JSON incrementally
                if (delta.partial_json) {
                    this.toolInputBuffer += delta.partial_json;
                }
                break;
        }

        return [];
    }

    /**
     * content_block_stop ends a content block.
     * For tool_use blocks, parse the accumulated input and update the tool_call.
     */
    private handleContentBlockStop(_event: any): StreamEvent[] {
        if (this.currentToolCallId && this.toolInputBuffer) {
            try {
                const params = JSON.parse(this.toolInputBuffer);
                // Emit an updated tool_call with parsed params
                const result: StreamEvent[] = [
                    {
                        type: "tool_call",
                        tool: this.currentToolName || "Unknown",
                        id: this.currentToolCallId,
                        params,
                    },
                ];
                this.currentToolCallId = null;
                this.currentToolName = null;
                this.toolInputBuffer = "";
                return result;
            } catch {
                // Failed to parse accumulated JSON - ignore
            }
        }

        this.currentToolCallId = null;
        this.currentToolName = null;
        this.toolInputBuffer = "";
        return [];
    }
}
