// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PermissionRequestEvent, SessionStats, StreamEvent } from "../types";
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
    // Map tool_use_id → tool name so tool_result can resolve the name
    // (Anthropic API does not include tool_name on tool_result blocks)
    private toolNameById: Map<string, string> = new Map();

    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];

        // Case 1: Already a StreamEvent (type is text/thinking/tool_call/tool_result/etc.)
        if (this.isStreamEvent(rawEvent)) {
            return [rawEvent as StreamEvent];
        }

        // Case 2: Wrapped in {"type":"stream_event","event":{...}}
        if (rawEvent.type === "stream_event" && rawEvent.event) {
            return this.translateInnerEvent(rawEvent.event);
        }

        // Case 3: Top-level "assistant" event (complete message)
        if (rawEvent.type === "assistant" && rawEvent.message) {
            // Skip partial snapshots — the same text arrives incrementally
            // via stream_event → content_block_delta before each snapshot,
            // so processing it here would produce duplicate document nodes.
            if (rawEvent.partial === true) return [];
            return this.handleAssistantMessage(rawEvent.message);
        }

        // Case 4: Top-level "user" event (tool results)
        if (rawEvent.type === "user" && rawEvent.message) {
            return this.handleUserMessage(rawEvent.message);
        }

        // Case 5a: Top-level "result" event — session complete with stats
        if (rawEvent.type === "result") {
            const stats: SessionStats = {};
            if (typeof rawEvent.cost_usd === "number") stats.cost_usd = rawEvent.cost_usd;
            if (typeof rawEvent.duration_ms === "number") stats.duration_ms = rawEvent.duration_ms;
            if (typeof rawEvent.num_turns === "number") stats.num_turns = rawEvent.num_turns;
            return [{ type: "session_end", stats }];
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
        this.toolNameById.clear();
    }

    /**
     * Stub. Claude Code CLI does not currently emit a structured
     * `permission_request` stream event; in non-bypass + non-interactive
     * mode it prompts y/n on the controlling tty. Detection strategy
     * (line-pattern recognition vs. waiting for an upstream event) is
     * decided in a later v1 PR — see
     * docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §9.1. This hook is
     * here so the rest of the system can wire to the right interface
     * shape today.
     */
    parsePermissionRequest(_raw: unknown): PermissionRequestEvent | null {
        return null;
    }

    private isStreamEvent(event: any): boolean {
        const streamTypes = [
            "text",
            "thinking",
            "tool_call",
            "tool_result",
            "agent_message",
            "user_message",
            "session_end",
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
     *
     * text/thinking blocks are NOT emitted here — they always arrive
     * incrementally via stream_event → content_block_delta before the
     * final assistant event lands, so processing them here would produce
     * duplicate document nodes (one from the streaming, one from this).
     *
     * Only tool_use blocks are extracted: they carry the final parsed
     * params that content_block_start + content_block_stop may not have
     * fully resolved yet. Node-ID deduplication in useAgentStream handles
     * the case where the tool node was already added via streaming.
     */
    private handleAssistantMessage(message: any): StreamEvent[] {
        if (!message || !Array.isArray(message.content)) return [];

        const events: StreamEvent[] = [];
        for (const block of message.content) {
            if (block.type === "tool_use") {
                this.currentToolCallId = block.id;
                this.currentToolName = block.name;
                if (block.id && block.name) {
                    this.toolNameById.set(block.id, block.name);
                }
                events.push({
                    type: "tool_call",
                    tool: block.name,
                    id: block.id,
                    params: typeof block.input === "object" && block.input !== null
                        ? block.input
                        : {},
                });
            }
        }
        return events;
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
                    const toolId = block.tool_use_id || `tool_${Date.now()}`;
                    results.push({
                        type: "tool_result",
                        tool: block.tool_name || this.toolNameById.get(toolId) || "Unknown",
                        id: toolId,
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
     * (these are the results of tool calls being fed back to Claude).
     *
     * Claude Code emits TWO views of the result:
     *   - `message.content[N].content` — human-readable string the model
     *     sees (concatenated stdout/stderr/exit prefix).
     *   - `event.tool_use_result` — sibling structured object with
     *     `stdout`, `stderr`, and optional `interrupted`.
     *
     * The structured shape is what BashOutputViewer (and other
     * per-tool viewers) expect — `result.stdout` / `result.stderr`.
     * Prefer it; fall back to the string-wrapped form when the
     * provider didn't emit a structured shape (e.g. non-bash tools,
     * older Claude Code).
     */
    private handleMessageStart(event: any): StreamEvent[] {
        const message = event.message;
        if (!message) return [];

        // Check for tool_result blocks in user messages
        if (message.role === "user" && Array.isArray(message.content)) {
            const results: StreamEvent[] = [];
            // `tool_use_result` is a sibling of `message` on the event,
            // not embedded per-block — it carries structured stdout/
            // stderr/exitCode for the tool that just ran. With a SINGLE
            // tool_result block in the message we can confidently apply
            // it; with multiple blocks (parallel tool use) the structured
            // result is unattributable (Claude doesn't include a
            // tool_use_id inside `tool_use_result`), so every block
            // falls back to the per-block string form. The dropped
            // structured shape just means BashOutputViewer renders
            // from `result.content` instead — same visible output,
            // missing the exitCode field.
            const structuredResult = event.tool_use_result;
            const toolResultBlocks = message.content.filter(
                (b: any) => b && b.type === "tool_result",
            );
            const canApplyStructured =
                toolResultBlocks.length === 1
                && structuredResult
                && typeof structuredResult === "object";
            for (const block of message.content) {
                if (block.type === "tool_result") {
                    const isError = block.is_error === true;
                    const toolId = block.tool_use_id || `tool_${Date.now()}`;
                    const fallback = typeof block.content === "string"
                        ? { content: block.content }
                        : block.content;
                    results.push({
                        type: "tool_result",
                        tool: block.tool_name || this.toolNameById.get(toolId) || "Unknown",
                        id: toolId,
                        status: isError ? "failed" : "success",
                        result: canApplyStructured ? structuredResult : fallback,
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
            // Record id→name so tool_result can resolve the tool name
            if (this.currentToolCallId && this.currentToolName) {
                this.toolNameById.set(this.currentToolCallId, this.currentToolName);
            }

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
