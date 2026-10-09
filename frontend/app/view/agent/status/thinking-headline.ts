// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The gist of what the model is thinking about, from its own thinking text:
 * a bold heading if it wrote one ("**Weighing the join rule**"), else its
 * first sentence. The live status says "Thinking: <that>" instead of only
 * "Thinking". Main agent only; a model whose thinking is redacted streams no
 * text, and the line stays "Thinking".
 *
 * docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.2.
 */

const MAX = 80;
/** Past this much text with no sentence end, take its start. */
const ENOUGH = 160;

function clip(s: string): string {
    const one = s.replace(/\s+/g, " ").trim();
    return one.length > MAX ? `${one.slice(0, MAX - 1)}…` : one;
}

/** The headline in a thinking block's text so far, or null if not yet clear. */
export function headlineOf(text: string): string | null {
    const t = text.trimStart();
    const bold = /^\*\*([^*\n]{3,})\*\*/.exec(t);
    if (bold) return clip(bold[1]);
    if (t.startsWith("**")) return null; // a heading still streaming
    const sentence = /^([\s\S]{12,}?[.?!])(\s|$)/.exec(t);
    if (sentence) return clip(sentence[1]);
    return t.length >= ENOUGH ? clip(t) : null;
}

interface StreamLine {
    type?: string;
    parent_tool_use_id?: unknown;
    event?: { type?: string; content_block?: { type?: string }; delta?: { type?: string; thinking?: unknown } };
}

/** Feed every stream line; returns a thinking block's headline once, when it becomes clear. */
export type ThinkingHeadlineTracker = (line: unknown) => string | null;

export function createThinkingHeadlineTracker(): ThinkingHeadlineTracker {
    let text = "";
    let found = false;
    return (raw) => {
        const line = (raw ?? {}) as StreamLine;
        if (line.parent_tool_use_id || line.type !== "stream_event") return null;
        const ev = line.event;
        if (ev?.type === "content_block_start") {
            text = "";
            found = ev.content_block?.type !== "thinking";
            return null;
        }
        if (ev?.type !== "content_block_delta" || ev.delta?.type !== "thinking_delta" || found) return null;
        if (typeof ev.delta.thinking !== "string") return null;
        text = (text + ev.delta.thinking).slice(0, 2 * ENOUGH);
        const headline = headlineOf(text);
        if (!headline) return null;
        found = true;
        return headline;
    };
}
