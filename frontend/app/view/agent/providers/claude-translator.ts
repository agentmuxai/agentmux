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

        // Case 4: Top-level "user" event (tool results). Pass the full
        // event so `tool_use_result` (sibling of `message`) is reachable.
        if (rawEvent.type === "user" && rawEvent.message) {
            return this.handleUserMessage(rawEvent.message, rawEvent.tool_use_result);
        }

        // Case 5: rate_limit_event — CLI is waiting on a 429, will retry
        if (rawEvent.type === "rate_limit_event") {
            return [{
                type: "provider_waiting",
                reason: "rate_limited" as const,
                retryAfterMs: typeof rawEvent.retry_after_ms === "number"
                    ? rawEvent.retry_after_ms
                    : null,
            }];
        }

        // Case 5a: Top-level "result" event — session complete with stats
        if (rawEvent.type === "result") {
            const events: StreamEvent[] = [];
            // Surface errors as an inline transcript node so the user sees the
            // failure reason in context. Covers both HTTP API errors (401/429/…
            // with api_error_status) and CLI-level/network errors (is_error:true
            // but no numeric api_error_status). Spec: SPEC_AGENT_ERROR_FRAMEWORK §P1.3
            if (rawEvent.is_error === true) {
                const code = typeof rawEvent.api_error_status === "number"
                    ? rawEvent.api_error_status
                    : 0; // 0 = non-HTTP error (network / CLI crash)
                // Field priority mirrors the backend classifier's
                // frame_error_text (agents/failure.rs): error.message →
                // error (string) → result → generic fallback. AgentMux's
                // own synthesized frames (e.g. the identity spawn gate's
                // error_during_execution refusal, agent_io.rs/input.rs)
                // carry their detail in `error.message`, NOT `result` —
                // reading only `result` rendered every gate refusal as a
                // bare "Agent encountered an error" with the actionable
                // "bind an account in the Armory" text silently dropped
                // (live repro: claudius v0.54.14 Agent1, 2026-08-09).
                const message = typeof rawEvent.result === "string"
                    ? rawEvent.result
                    : typeof rawEvent.error?.message === "string"
                        ? rawEvent.error.message
                        : typeof rawEvent.error === "string"
                            ? rawEvent.error
                            : code > 0 ? `API error ${code}` : "Agent encountered an error";
                events.push({ type: "error_result", code, message });
            }
            const stats: SessionStats = {};
            if (typeof rawEvent.cost_usd === "number") stats.cost_usd = rawEvent.cost_usd;
            if (typeof rawEvent.duration_ms === "number") stats.duration_ms = rawEvent.duration_ms;
            if (typeof rawEvent.num_turns === "number") stats.num_turns = rawEvent.num_turns;
            // input_tokens is only the uncached prompt; cache_creation/cache_read
            // carry the rest of the real prompt size.
            const usage = rawEvent.usage;
            if (usage && typeof usage === "object") {
                const input =
                    (usage.input_tokens ?? 0) +
                    (usage.cache_creation_input_tokens ?? 0) +
                    (usage.cache_read_input_tokens ?? 0);
                const output = usage.output_tokens ?? 0;
                if (input > 0) stats.input_tokens = input;
                if (output > 0) stats.output_tokens = output;
            }
            events.push({ type: "session_end", stats });
            return events;
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
            "provider_waiting",
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
        let hasToolUse = false;
        let hasText = false;
        for (const block of message.content) {
            if (block.type === "tool_use") {
                hasToolUse = true;
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
            if (block.type === "text" && typeof block.text === "string" && block.text.trim().length > 0) {
                hasText = true;
            }
        }
        // In persistent/interactive mode the process never exits between turns,
        // so "result" is only emitted at session teardown. Detect per-turn
        // completion here instead. A message only counts as the real final
        // response of a turn once it contains actual explanation text — not
        // merely "no tool_use". A thinking-only or empty message is a
        // transitional state (e.g. the model is still assembling its
        // response, or a message boundary landed between a tool result and
        // the model's real reply); firing session_end there settles the UI
        // to "Worked" a beat before the real explanation streams in, then
        // immediately reopens it — see
        // SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md. In subprocess
        // (--print) mode the "result" event fires a duplicate TurnEnd, which
        // is a no-op (first-done-wins in the reducer).
        //
        // stop_reason check (2026-08-08 recurrence of the same symptom the
        // text gate was built for): the CLI splits each API message into ONE
        // assistant frame PER content block — a real session showed zero
        // combined [text, tool_use] frames across 1,790 assistant frames.
        // Narration text preceding a tool call therefore arrives as its own
        // text-only frame (hasText, no tool_use) and passed the gate, ending
        // the turn mid-work — 377 of 409 text-only frames in that session
        // were this mid-turn kind, all stamped stop_reason "tool_use", while
        // genuine finals carry "end_turn"/"stop_sequence". Only suppress on
        // an AFFIRMATIVE "tool_use" — null/undefined (an older CLI that
        // doesn't stamp it) and every terminal reason still end the turn, so
        // this cannot reintroduce #1757's stuck-forever failure; the 180s
        // liveness watchdog remains the backstop either way.
        if (!hasToolUse && hasText && message.stop_reason !== "tool_use") {
            events.push({ type: "session_end", stats: {} });
        }
        return events;
    }

    /**
     * Top-level "user" event contains tool_result blocks. The caller
     * forwards `event.tool_use_result` here so the structured shape
     * (`stdout/stderr/interrupted`) reaches `buildToolResults` —
     * keeps this path symmetric with the `message_start` user-role
     * path.
     */
    private handleUserMessage(message: any, structuredResult?: any): StreamEvent[] {
        const content = message.content;
        if (!content) return [];

        // Handle string content
        if (typeof content === "string") {
            return [{ type: "user_message", message: content, timestamp: Date.now() }];
        }

        // Handle array content with tool_result blocks
        if (Array.isArray(content)) {
            const results = this.buildToolResults(content, structuredResult);
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
            return this.buildToolResults(message.content, event.tool_use_result);
        }

        return [];
    }

    /**
     * Shared tool_result builder used by both entry points
     * (`message_start` with role:user, and top-level `user` events).
     *
     * `tool_use_result` is a sibling of `message` on the event — it
     * carries structured `{ stdout, stderr, interrupted }` for the
     * tool that just ran. With a SINGLE tool_result block in the
     * message we can confidently apply it; with multiple blocks
     * (parallel tool use) the structured result is unattributable
     * (Claude doesn't include a `tool_use_id` inside it), so every
     * block falls back to the per-block string form.
     *
     * Note: Claude's `tool_use_result` carries no `exitCode` — only
     * stdout/stderr/interrupted. The wrapper encodes the exit code
     * as an `<exited N in Ts>` prefix in stdout; BashOutputViewer
     * parses it back out at render time so failed bash commands stay
     * visible even when stderr is empty.
     */
    private buildToolResults(content: any[], structuredResult: any): StreamEvent[] {
        const results: StreamEvent[] = [];
        const toolResultBlocks = content.filter(
            (b: any) => b && b.type === "tool_result",
        );
        // Only apply the terminal-style sibling (stdout/stderr/interrupted) when it
        // is actually terminal-shaped. Web search and other non-bash tools may carry
        // a structuredResult sibling in a different shape; applying it would discard
        // the real block.content (e.g. the web_search_result string/array).
        const isTerminalShaped = (r: unknown): boolean =>
            Boolean(r && typeof r === "object" && !Array.isArray(r)
                && ("stdout" in (r as object) || "stderr" in (r as object) || "interrupted" in (r as object)));
        const canApplyStructured =
            toolResultBlocks.length === 1
            && isTerminalShaped(structuredResult);
        for (const block of content) {
            if (block.type === "tool_result") {
                const isError = block.is_error === true;
                const toolId = block.tool_use_id || `tool_${Date.now()}`;
                // Only apply the structured sibling result when block.content is a string
                // (the bash stdout path). If block.content is already an array/object
                // (e.g. web_search_result blocks), use it directly — applying the
                // terminal-shaped { stdout, stderr } sibling would discard the real data.
                const blockContentIsString = typeof block.content === "string";
                const fallback = blockContentIsString
                    ? { content: block.content }
                    : block.content;
                const useStructured = canApplyStructured && blockContentIsString;
                results.push({
                    type: "tool_result",
                    tool: block.tool_name || this.toolNameById.get(toolId) || "Unknown",
                    id: toolId,
                    status: isError ? "failed" : "success",
                    result: useStructured ? structuredResult : fallback,
                });
            }
        }
        return results;
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
