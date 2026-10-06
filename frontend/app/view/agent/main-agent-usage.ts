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
