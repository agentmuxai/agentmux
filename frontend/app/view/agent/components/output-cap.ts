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

/** Max rendered characters of a single text body. Bounds a payload that fits
 *  the line budget but is one enormous line (minified JSON, base64, a long
 *  compiler line). ~1 MB: comfortably above any real multi-line output, well
 *  below the multi-MB blobs that bloat the conversation DOM. */
export const MAX_TOOL_OUTPUT_CHARS = 1_000_000;

export interface CappedText {
    text: string;
    /** Lines dropped (0 when under budget). */
    hiddenLines: number;
}

/**
 * Cap a text body to `max` lines, keeping the head or the tail. Logs and
 * command output use "tail" (the latest matters); file/diff content uses
 * "head" (read top-down). Exactly-at-budget is not considered capped. The
 * result is additionally bounded to MAX_TOOL_OUTPUT_CHARS so one enormous
 * line (minified JSON, base64) can't render an unbounded <pre>.
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
    let hiddenLines = 0;
    let out = text;
    if (body.length > max) {
        const kept = from === "tail" ? body.slice(body.length - max) : body.slice(0, max);
        hiddenLines = body.length - max;
        out = kept.join("\n");
    }
    // Character bound: a payload within the line budget can still be enormous
    // as one long line (minified JSON, base64). Trim by chars with a visible
    // marker so the rendered <pre> stays bounded regardless of line count.
    if (out.length > MAX_TOOL_OUTPUT_CHARS) {
        out = from === "tail"
            ? "…(truncated)\n" + out.slice(out.length - MAX_TOOL_OUTPUT_CHARS)
            : out.slice(0, MAX_TOOL_OUTPUT_CHARS) + "\n…(truncated)";
    }
    return { text: out, hiddenLines };
}

/**
 * Trim a single string to `max` characters (keeping the tail) with a visible
 * marker. For the streamed-chunk path, which renders raw chunk content rather
 * than routing through capText — a single multi-MB one-line chunk would
 * otherwise render an unbounded <pre> even though it is under the line budget.
 */
export function capChars(text: string, max: number = MAX_TOOL_OUTPUT_CHARS): string {
    if (text.length <= max) return text;
    return "…(truncated)\n" + text.slice(text.length - max);
}

export interface CappedChunks<T> {
    chunks: ReadonlyArray<T>;
    /** Lines retained in the rendered window (== total when under budget). */
    keptLines: number;
}

/**
 * Cap an append-only chunk list (one <pre> per chunk) to the tail whose
 * cumulative line count first reaches `maxLines`. Walks ONLY the kept tail —
 * O(kept), never the dropped prefix. Retained chunk objects keep their
 * identity, so a `<For>` keyed by reference only churns at the window
 * boundary. A single oversized boundary chunk is line-trimmed to its tail
 * (not kept whole) so one chunk can't render an unbounded <pre>.
 *
 * Returns the kept line count; pair it with a running total
 * (createChunkCapper) to derive the hidden-line count without ever rescanning
 * the dropped prefix.
 */
export function capChunksByLines<T extends { content: string }>(
    chunks: ReadonlyArray<T>,
    maxLines: number = MAX_TOOL_OUTPUT_LINES,
): CappedChunks<T> {
    let budget = maxLines;
    let i = chunks.length - 1;
    for (; i >= 0; i--) {
        // Each chunk renders one <pre>, so it costs at least one line of
        // budget — otherwise a run of empty chunks would retain unbounded nodes.
        const n = Math.max(1, countLines(chunks[i].content));
        if (n > budget) break; // this chunk overflows the remaining budget
        budget -= n;
    }
    // Under budget: keep everything (kept == total == maxLines - budget).
    if (i < 0) return { chunks, keptLines: maxLines - budget };
    // Capping: the window holds exactly `maxLines` lines (whole tail chunks
    // plus, if the budget didn't land on a chunk boundary, a trimmed boundary).
    if (budget <= 0) {
        return { chunks: chunks.slice(i + 1), keptLines: maxLines };
    }
    const trimmed = capText(chunks[i].content, budget, "tail");
    return {
        chunks: [{ ...chunks[i], content: trimmed.text } as T, ...chunks.slice(i + 1)],
        keptLines: maxLines,
    };
}

/** A capped chunk window plus how many earlier lines are hidden. */
export interface CappedChunkView<T> {
    chunks: ReadonlyArray<T>;
    hiddenLines: number;
}

/**
 * Stateful capper for an append-only chunk stream. Tracks the total line
 * count incrementally — each call scans only the chunks appended since the
 * previous call — so the hidden-line count (total − kept) never rescans the
 * growing dropped prefix (which would reintroduce O(n^2) over a long stream).
 * One instance per stream; on the rare reset (the array shrinks) it recounts.
 */
export function createChunkCapper(maxLines: number = MAX_TOOL_OUTPUT_LINES) {
    let total = 0;
    let counted = 0;
    let anchor: { content: string } | undefined;
    return function cap<T extends { content: string }>(chunks: ReadonlyArray<T>): CappedChunkView<T> {
        // Reset the running total unless this is an append-only continuation of
        // the same stream. A recycled ChunkList can be handed a different
        // tool's chunks — even with the same length — so anchor on the first
        // chunk's identity (stable under append), not just the array length.
        if (chunks.length < counted || chunks[0] !== anchor) {
            total = 0;
            counted = 0;
            anchor = chunks[0];
        }
        // Match capChunksByLines: each chunk costs >= 1 line (it renders a <pre>).
        for (; counted < chunks.length; counted++) total += Math.max(1, countLines(chunks[counted].content));
        const r = capChunksByLines(chunks, maxLines);
        return { chunks: r.chunks, hiddenLines: Math.max(0, total - r.keptLines) };
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
