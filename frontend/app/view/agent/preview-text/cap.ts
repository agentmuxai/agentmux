// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Size caps, applied once, with what was cut kept as data
 * (REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md §5.1 step 7). The old
 * `capText` put a "…(truncated)" line into the text itself, where it took part
 * in dedent and highlighting.
 */

import { MAX_TOOL_OUTPUT_CHARS, MAX_TOOL_OUTPUT_LINES } from "../components/output-cap";
import type { PreviewDoc } from "./types";

export { MAX_TOOL_OUTPUT_CHARS, MAX_TOOL_OUTPUT_LINES };

/** Keep at most `maxChars` characters of `text`, from the end it's read from.
 *  Applied before splitting so a huge single line can't be processed whole. */
export function capChars(text: string, from: "head" | "tail", maxChars: number = MAX_TOOL_OUTPUT_CHARS): string {
    if (text.length <= maxChars) return text;
    return from === "head" ? text.slice(0, maxChars) : text.slice(text.length - maxChars);
}

/** Keep at most `maxLines` lines of `doc`, from the end it's read from, and
 *  record how many were left out. */
export function capLines<T extends PreviewDoc>(
    doc: T,
    from: "head" | "tail",
    maxLines: number = MAX_TOOL_OUTPUT_LINES
): T {
    const n = doc.lines.length;
    if (n <= maxLines) return doc;
    const lines = from === "head" ? doc.lines.slice(0, maxLines) : doc.lines.slice(n - maxLines);
    return { ...doc, lines, hidden: { count: n - maxLines, from } };
}

/** The same cap on raw lines, for builders that must cap before they process
 *  (a Read's dedent should only see the lines that are shown). */
export function capRawLines(
    lines: string[],
    from: "head" | "tail",
    maxLines: number = MAX_TOOL_OUTPUT_LINES
): { lines: string[]; hidden: number } {
    if (lines.length <= maxLines) return { lines, hidden: 0 };
    return {
        lines: from === "head" ? lines.slice(0, maxLines) : lines.slice(lines.length - maxLines),
        hidden: lines.length - maxLines,
    };
}
