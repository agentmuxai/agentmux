// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Token usage, and the model id, from one raw stream line — for the MAIN agent
 * only.
 *
 * The CLI's stream carries the main agent's lines and, interleaved, those of
 * every subagent it runs: those carry a `parent_tool_use_id`
 * (see model-turn-signal.ts, which already ignores them for the same reason).
 * A subagent has its own context and often its own model (a cheap one for
 * lookups), so reading ITS `message_start` as the pane's usage made the context
 * meter jump to the subagent's fill, and learned the pane's context window from
 * the subagent's model — a 200K Haiku subagent shrinking the meter of a 1M
 * Sonnet pane for as long as it ran.
 *
 * One API call's prompt is reported twice: by the call's `message_start`
 * (a `stream_event` with partial messages on) and by the `assistant` frame(s)
 * that carry its content — same message id, same input counts (verified on CLI
 * 2.1.288). Both are read, so the figure survives a stream without partial
 * messages and a transcript whose stream events were dropped; `messageId` lets
 * a caller count each call once. This is the ONLY source of "tokens in
 * context": a `result` frame's `usage` sums every call of the turn and must
 * never stand in for it (store/agent-pane-state/context-reading.ts).
 */

/** The fields of a raw stream line this reads; everything else is ignored. */
interface StreamLineShape {
    type?: string;
    parent_tool_use_id?: string | null;
    event?: { type?: string; message?: MessageShape; usage?: { output_tokens?: number } };
    message?: MessageShape;
    usage?: { output_tokens?: number };
    [field: string]: unknown;
}
interface MessageShape {
    id?: string;
    model?: string;
    /** A `user` frame's blocks (tool results), for mainAgentRequestStarted. */
    content?: unknown;
    usage?: {
        input_tokens?: number;
        cache_creation_input_tokens?: number;
        cache_read_input_tokens?: number;
    };
}

export type MainAgentUsage =
    | {
          kind: "in";
          /** The whole prompt: uncached + cache-creation + cache-read tokens. */
          input: number;
          /** The uncached part alone. */
          freshInput: number;
          cacheCreation: number;
          cacheRead: number;
          /** The resolved model id (e.g. "claude-opus-5-5"), when the line names one. */
          model: string | undefined;
          /** The API message id (`msg_…`) the call is reported under, when present. */
          messageId?: string;
      }
    | { kind: "out"; output: number };

/**
 * Whether a stream in `outputFormat` carries Claude Code's per-call usage
 * (`message_start` / `assistant` frames) that `mainAgentUsage` reads. Other
 * providers' frames have other shapes and meanings; their panes show no
 * context reading rather than one whose meaning was never checked.
 */
export function readsMainAgentUsage(outputFormat: string): boolean {
    return outputFormat === "claude-stream-json";
}

export function mainAgentUsage(rawEvent: StreamLineShape): MainAgentUsage | null {
    if (rawEvent.parent_tool_use_id) return null;
    const inner = rawEvent.type === "stream_event" ? rawEvent.event : rawEvent;
    // An `assistant` frame counts only with its API message id: that is what
    // ties it to one call (and to that call's message_start), so the call is
    // counted once.
    const isCallFrame =
        inner?.type === "message_start" ||
        (inner?.type === "assistant" && typeof inner.message?.id === "string" && inner.message.id !== "");
    if (isCallFrame) {
        const u = inner.message?.usage;
        const freshInput = u?.input_tokens;
        if (freshInput == null) return null;
        const cacheCreation = u?.cache_creation_input_tokens ?? 0;
        const cacheRead = u?.cache_read_input_tokens ?? 0;
        // Claude Code writes `assistant` frames of its own (an interrupted
        // turn, an API error) under the model "<synthetic>" with all-zero
        // usage: no API call happened, so they say nothing about the context.
        if (freshInput + cacheCreation + cacheRead <= 0 || inner.message?.model === "<synthetic>") return null;
        return {
            kind: "in",
            input: freshInput + cacheCreation + cacheRead,
            freshInput,
            cacheCreation,
            cacheRead,
            model: inner.message?.model,
            messageId: typeof inner.message?.id === "string" && inner.message.id ? inner.message.id : undefined,
        };
    }
    if (inner?.type === "message_delta") {
        const output = inner.usage?.output_tokens;
        return output != null ? { kind: "out", output } : null;
    }
    return null;
}

/**
 * Characters of model output a main-agent `content_block_delta` streams: text,
 * thinking, and tool-call input, which Claude Code's own spinner counts (at
 * four a token) for its turn token counter. A `signature_delta` is not output
 * and isn't counted. 0 for anything else, and for a subagent's lines.
 * SPEC_AGENT_TURN_TOKEN_COUNTER_CLAUDE_CONVENTION_2026_10_07.md §2.
 */
export function mainAgentStreamedChars(rawEvent: StreamLineShape): number {
    if (rawEvent.parent_tool_use_id || rawEvent.type !== "stream_event") return 0;
    const inner = rawEvent.event as { type?: string; delta?: { type?: string; text?: unknown; thinking?: unknown; partial_json?: unknown } } | undefined;
    if (inner?.type !== "content_block_delta") return 0;
    const d = inner.delta;
    const s = d?.type === "text_delta" ? d.text : d?.type === "thinking_delta" ? d.thinking : d?.type === "input_json_delta" ? d.partial_json : null;
    return typeof s === "string" ? s.length : 0;
}

/** What a main-agent `content_block_delta` streams: the model's reply text,
 *  its thinking, or a tool call's input. For the live status's phase. */
export type StreamedKind = "text" | "thinking" | "tool_input";

/** The kind of output a line streams, when it is a main-agent delta that counts. */
export function mainAgentStreamedKind(rawEvent: StreamLineShape): StreamedKind | null {
    if (rawEvent.parent_tool_use_id || rawEvent.type !== "stream_event") return null;
    const inner = rawEvent.event as { type?: string; delta?: { type?: string } } | undefined;
    if (inner?.type !== "content_block_delta") return null;
    switch (inner.delta?.type) {
        case "text_delta":
            return "text";
        case "thinking_delta":
            return "thinking";
        case "input_json_delta":
            return "tool_input";
        default:
            return null;
    }
}

/**
 * A main-agent `user` frame carrying tool results: the CLI is sending them
 * back, so a request is in flight until the next call's `message_start`
 * (Claude Code's `requesting` mode, the ↑ arrow).
 */
export function mainAgentRequestStarted(rawEvent: StreamLineShape): boolean {
    if (rawEvent.parent_tool_use_id || rawEvent.type !== "user") return false;
    const content = rawEvent.message?.content;
    return Array.isArray(content) && content.some((b) => (b as { type?: unknown } | null)?.type === "tool_result");
}
