// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tab expansion, the one place a tab's width is decided
 * (REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md §3.2, §5.1 step 5).
 *
 * Tabs used to reach the DOM and be sized by CSS `tab-size`, which counted the
 * line-number gutter and diff marker as part of the line and only applied in
 * some previews. Expanding here, against the line's own start, makes a tab the
 * same width wherever the line is shown.
 */

import { PREVIEW_INDENT_UNIT } from "../components/dedent";

/** Columns per tab for code and diffs: one indentation level, the width
 *  space indentation is narrowed to, so a tab-indented file and a
 *  space-indented one render at the same width. */
export const CODE_TAB_WIDTH = PREVIEW_INDENT_UNIT;

/** Columns per tab for command output and prose: what the programs that
 *  wrote it assumed (`ls -l`, TSV, `git log --format=%x09`). */
export const OUTPUT_TAB_WIDTH = 8;

/**
 * Replace every tab in `line` with the spaces that reach the next tab stop,
 * counting columns from the start of `line`. Code points count as one column
 * each. Returns `line` itself when it has no tab.
 */
export function expandTabs(line: string, width: number): string {
    if (!line.includes("\t")) return line;
    let out = "";
    let col = 0;
    for (const ch of line) {
        if (ch === "\t") {
            const n = width - (col % width);
            out += " ".repeat(n);
            col += n;
        } else {
            out += ch;
            col++;
        }
    }
    return out;
}
