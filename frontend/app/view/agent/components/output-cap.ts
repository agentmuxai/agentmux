// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tool-output render caps — bound the DOM a single tool's output body
 * contributes to the conversation, so a long tool call can't tank scroll.
 *
 * Interim mitigation per SPEC_TOOL_OUTPUT_CAP_2026_05_30.md: the agent
 * conversation is virtualized, but a single tool's output body is not — a
 * long Bash run is otherwise one <pre> per chunk (or one enormous <pre>),
 * thousands of in-flow nodes. We cap what is *rendered inline*; the full
 * output stays in ToolNode state and is reachable via "open in pane".
 */

/** Max rendered lines of a single text body (Bash stdout, Read content, …). */
export const MAX_TOOL_OUTPUT_LINES = 1000;

export interface CappedText {
    text: string;
    /** Lines dropped (0 when under budget). */
    hiddenLines: number;
}

/**
 * Cap a text body to `max` lines, keeping the head or the tail. Logs and
 * command output use "tail" (the latest matters); file/diff content uses
 * "head" (read top-down). Exactly-at-budget is not considered capped.
 */
export function capText(
    text: string,
    max: number = MAX_TOOL_OUTPUT_LINES,
    from: "head" | "tail" = "tail",
): CappedText {
    if (!text) return { text: text ?? "", hiddenLines: 0 };
    const lines = text.split("\n");
    if (lines.length <= max) return { text, hiddenLines: 0 };
    const kept = from === "tail" ? lines.slice(lines.length - max) : lines.slice(0, max);
    return { text: kept.join("\n"), hiddenLines: lines.length - max };
}

export interface CappedChunks<T> {
    chunks: ReadonlyArray<T>;
    /** Chunks dropped off the front (0 when under budget). */
    hiddenChunks: number;
}

/**
 * Cap an append-only chunk list (one <pre> per chunk) to the tail whose
 * cumulative line count first reaches `maxLines`. Walks only the kept tail
 * — O(kept), not O(all) — so it is cheap to call on every streamed append.
 * Retained chunk objects keep their identity, so a `<For>` keyed by
 * reference only churns at the window boundary.
 *
 * A single oversized tail chunk is kept whole (one <pre>, one node — far
 * cheaper than the many-node case this guards against).
 */
export function capChunksByLines<T extends { content: string }>(
    chunks: ReadonlyArray<T>,
    maxLines: number = MAX_TOOL_OUTPUT_LINES,
): CappedChunks<T> {
    let budget = maxLines;
    let firstKept = chunks.length;
    for (let i = chunks.length - 1; i >= 0; i--) {
        firstKept = i;
        budget -= countLines(chunks[i].content);
        if (budget <= 0) break;
    }
    if (firstKept <= 0) return { chunks, hiddenChunks: 0 };
    return { chunks: chunks.slice(firstKept), hiddenChunks: firstKept };
}

/** Visual line count of a string (newline-delimited), allocation-free. */
function countLines(s: string): number {
    if (!s) return 0;
    let n = 1;
    for (let i = 0; i < s.length; i++) {
        if (s.charCodeAt(i) === 10 /* \n */) n++;
    }
    return n;
}
