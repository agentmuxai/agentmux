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
      }
    | { kind: "out"; output: number };

export function mainAgentUsage(rawEvent: StreamLineShape): MainAgentUsage | null {
    if (rawEvent.parent_tool_use_id) return null;
    const inner = rawEvent.type === "stream_event" ? rawEvent.event : rawEvent;
    if (inner?.type === "message_start") {
        const u = inner.message?.usage;
        const freshInput = u?.input_tokens;
        if (freshInput == null) return null;
        const cacheCreation = u?.cache_creation_input_tokens ?? 0;
        const cacheRead = u?.cache_read_input_tokens ?? 0;
        return {
            kind: "in",
            input: freshInput + cacheCreation + cacheRead,
            freshInput,
            cacheCreation,
            cacheRead,
            model: inner.message?.model,
        };
    }
    if (inner?.type === "message_delta") {
        const output = inner.usage?.output_tokens;
        return output != null ? { kind: "out", output } : null;
    }
    return null;
}
