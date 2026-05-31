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
    // A trailing newline yields a phantom empty final segment — exclude it so
    // exactly-`max` lines followed by "\n" is not treated as over budget.
    const body = lines.length > 1 && lines[lines.length - 1] === "" ? lines.slice(0, -1) : lines;
    if (body.length <= max) return { text, hiddenLines: 0 };
    const kept = from === "tail" ? body.slice(body.length - max) : body.slice(0, max);
    return { text: kept.join("\n"), hiddenLines: body.length - max };
}

export interface CappedChunks<T> {
    chunks: ReadonlyArray<T>;
    /** Total lines hidden — dropped whole chunks plus any trimmed boundary. */
    hiddenLines: number;
}

/**
 * Cap an append-only chunk list (one <pre> per chunk) to the tail whose
 * cumulative line count first reaches `maxLines`. Retained chunk objects keep
 * their identity, so a `<For>` keyed by reference only churns at the window
 * boundary. If a single boundary chunk alone exceeds the remaining budget it
 * is line-trimmed to its tail (not kept whole), so one chunk that batches a
 * huge output can't render an unbounded <pre>.
 *
 * Reports the exact number of hidden lines (dropped chunks + the trimmed
 * boundary). Counting the dropped remainder is a cheap char scan — not a
 * re-parse — so it stays inexpensive even on large output.
 */
export function capChunksByLines<T extends { content: string }>(
    chunks: ReadonlyArray<T>,
    maxLines: number = MAX_TOOL_OUTPUT_LINES,
): CappedChunks<T> {
    let budget = maxLines;
    let i = chunks.length - 1;
    for (; i >= 0; i--) {
        const n = countLines(chunks[i].content);
        if (n > budget) break; // this chunk overflows the remaining budget
        budget -= n;
    }
    if (i < 0) return { chunks, hiddenLines: 0 };
    // Lines from every chunk fully dropped off the front.
    let hiddenLines = 0;
    for (let j = 0; j < i; j++) hiddenLines += countLines(chunks[j].content);
    if (budget <= 0) {
        // No room left for the boundary chunk — drop it whole too.
        return { chunks: chunks.slice(i + 1), hiddenLines: hiddenLines + countLines(chunks[i].content) };
    }
    // The boundary chunk alone exceeds the budget — keep only its last `budget`
    // lines so one oversized chunk can't render unbounded.
    const trimmed = capText(chunks[i].content, budget, "tail");
    return {
        chunks: [{ ...chunks[i], content: trimmed.text } as T, ...chunks.slice(i + 1)],
        hiddenLines: hiddenLines + trimmed.hiddenLines,
    };
}

/** Visual line count of a string (newline-delimited), allocation-free. */
function countLines(s: string): number {
    if (!s) return 0;
    let n = 1;
    for (let i = 0; i < s.length; i++) {
        if (s.charCodeAt(i) === 10 /* \n */) n++;
    }
    // A trailing newline terminates the last line — don't count a phantom.
    if (s.charCodeAt(s.length - 1) === 10) n--;
    return n;
}
