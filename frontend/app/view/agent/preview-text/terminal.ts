// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Terminal text to clean lines (REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md
 * §5.1 step 2): what a terminal would have shown, not the bytes that drew it.
 *
 * - SGR colour codes become {@link StyleSpan}s, with the colour carried from
 *   one line to the next as a terminal does (the old `AnsiLine` reset it on
 *   every line).
 * - Other escape sequences (cursor moves, titles, mode switches) are dropped;
 *   `ESC[K` (erase to end of line) and `ESC[<n>G` (go to column) are applied.
 * - `\r` returns to column 0 and later text overwrites, so a progress bar that
 *   redraws its line shows its last state. `\r\n` is a line break. `\b` steps
 *   back one column.
 * - Tabs move to the next multiple of `tabWidth`, counted from column 0.
 * - Other control characters are dropped.
 * - The text `^[[<row>;<col>R` at the very start is removed: bashwrap's
 *   cursor-position reply, echoed by a Unix PTY into results recorded before
 *   #4508 fixed it at the source.
 */

import { makeInitialState, stateToClasses, updateStateWithCodes, type AnsiState } from "@/element/ansiline";
import type { PreviewLine, StyleSpan } from "./types";

const ECHOED_CURSOR_REPORT_RE = /^(?:\^\[\[\d+;\d+R)+/;

/** Characters that need the full decoder; text without any is split as is. */
const NEEDS_DECODE_RE = /[\x00-\x08\x0b-\x1f\x7f\t]/;

interface Cell {
    ch: string;
    classes: string;
}

/** Decode `text` into lines. `tabWidth` is in columns. */
export function decodeTerminal(text: string, tabWidth: number): PreviewLine[] {
    const src = text.replace(ECHOED_CURSOR_REPORT_RE, "");
    // Fast path: plain text, the common case for tool output.
    if (!NEEDS_DECODE_RE.test(src)) return src.split("\n").map((line) => ({ text: line }));

    const lines: PreviewLine[] = [];
    const state: AnsiState = makeInitialState();
    let classes = "";
    let cells: Cell[] = [];
    let col = 0;
    let colored = false;

    const put = (ch: string) => {
        while (cells.length < col) cells.push({ ch: " ", classes: "" });
        cells[col] = { ch, classes };
        if (classes) colored = true;
        col++;
    };
    const endLine = () => {
        lines.push(toLine(cells, colored));
        cells = [];
        col = 0;
        colored = false;
    };

    const chars = Array.from(src);
    for (let i = 0; i < chars.length; i++) {
        const ch = chars[i];
        if (ch === "\n") {
            endLine();
        } else if (ch === "\r") {
            if (chars[i + 1] === "\n") continue; // CRLF: the \n ends the line
            col = 0;
        } else if (ch === "\t") {
            const n = tabWidth - (col % tabWidth);
            for (let k = 0; k < n; k++) put(" ");
        } else if (ch === "\b") {
            col = Math.max(0, col - 1);
        } else if (ch === "\x1b") {
            i = escape(chars, i, (final, params) => {
                if (final === "m") {
                    updateStateWithCodes(state, params === "" ? [0] : params.split(";").map(Number));
                    classes = stateToClasses(state);
                } else if (final === "K") {
                    const mode = params === "" ? 0 : Number(params);
                    if (mode === 0) cells.length = Math.min(cells.length, col);
                    else if (mode === 2) cells = [];
                } else if (final === "G") {
                    col = Math.max(0, (params === "" ? 1 : Number(params)) - 1);
                }
            });
        } else if (ch < " " || ch === "\x7f") {
            // other C0 controls (bell, NUL, …): not content
        } else {
            put(ch);
        }
    }
    endLine();
    return lines;
}

/**
 * Consume the escape sequence starting at `chars[i]` (an ESC) and return the
 * index of its last character. A CSI sequence's final byte and parameters are
 * passed to `onCsi`; OSC (`ESC ] … BEL` or `ESC ] … ESC \`) and two-character
 * sequences are skipped.
 */
function escape(chars: string[], i: number, onCsi: (final: string, params: string) => void): number {
    const next = chars[i + 1];
    if (next === "[") {
        let j = i + 2;
        let params = "";
        while (j < chars.length && chars[j] >= "\x30" && chars[j] <= "\x3f") params += chars[j++];
        while (j < chars.length && chars[j] >= "\x20" && chars[j] <= "\x2f") j++; // intermediates
        if (j < chars.length) onCsi(chars[j], params);
        return Math.min(j, chars.length - 1);
    }
    if (next === "]") {
        let j = i + 2;
        while (j < chars.length) {
            if (chars[j] === "\x07") return j;
            if (chars[j] === "\x1b" && chars[j + 1] === "\\") return j + 1;
            j++;
        }
        return chars.length - 1;
    }
    return Math.min(i + 1, chars.length - 1);
}

function toLine(cells: Cell[], colored: boolean): PreviewLine {
    const text = cells.map((c) => c.ch).join("");
    if (!colored) return { text };
    const spans: StyleSpan[] = [];
    for (const c of cells) {
        const last = spans[spans.length - 1];
        if (last && last.classes === c.classes) last.text += c.ch;
        else spans.push({ text: c.ch, classes: c.classes });
    }
    return spans.some((s) => s.classes) ? { text, spans } : { text };
}
