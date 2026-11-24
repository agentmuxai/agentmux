// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Delimited text (CSV, TSV and their kin) as rows of cells, for the Table
 * viewer. RFC 4180: a cell may be quoted, a quote inside one is doubled, a
 * quoted cell may hold the delimiter and line breaks; lines end in LF, CRLF
 * or CR. Pure.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4 (phase 5).
 */

export type Delimiter = "," | "\t" | ";" | "|";

/** What each delimiter is called in the viewer bar. */
export const DELIMITER_NAMES: Record<Delimiter, string> = { ",": "comma", "\t": "tab", ";": "semicolon", "|": "pipe" };

const CANDIDATES: readonly Delimiter[] = [",", "\t", ";", "|"];

/** Lines read to tell the delimiter. */
const SNIFF_LINES = 10;

/**
 * The delimiter of `text`: the one that splits the first lines into the same number of cells (at least two), the
 * most cells winning, a comma on a tie. A hint (from the file's type) wins when it fits the first line.
 */
export function sniffDelimiter(text: string, hint?: Delimiter): Delimiter {
    const lines = text.slice(0, 64 * 1024).split(/\r\n|\n|\r/, SNIFF_LINES).filter((l) => l !== "");
    const score = (d: Delimiter): number => {
        let least = Infinity;
        for (const line of lines) least = Math.min(least, line.split(d).length - 1);
        return Number.isFinite(least) ? least : 0;
    };
    if (hint && score(hint) > 0) return hint;
    let best: Delimiter = ",";
    let bestScore = 0;
    for (const d of CANDIDATES) {
        const s = score(d);
        if (s > bestScore) {
            best = d;
            bestScore = s;
        }
    }
    return best;
}

/** `text` as rows of cells. A line break ending the text starts no row; a blank line in it is a row of one empty cell. */
export function parseDelimited(text: string, delimiter: Delimiter): string[][] {
    const rows: string[][] = [];
    let row: string[] = [];
    let cell = "";
    let quoted = false;
    let i = text.charCodeAt(0) === 0xfeff ? 1 : 0;
    const n = text.length;
    while (i < n) {
        const c = text[i];
        if (quoted) {
            // Up to the next quote in one step: a quoted cell can be long.
            const q = text.indexOf('"', i);
            if (q < 0) {
                cell += text.slice(i);
                i = n;
            } else if (text[q + 1] === '"') {
                cell += text.slice(i, q + 1);
                i = q + 2;
            } else {
                cell += text.slice(i, q);
                quoted = false;
                i = q + 1;
            }
            continue;
        }
        if (c === '"' && cell === "") {
            quoted = true;
            i++;
        } else if (c === delimiter) {
            row.push(cell);
            cell = "";
            i++;
        } else if (c === "\n" || c === "\r") {
            row.push(cell);
            rows.push(row);
            row = [];
            cell = "";
            i += c === "\r" && text[i + 1] === "\n" ? 2 : 1;
        } else {
            // Up to the next character that means something: most cells are plain.
            let j = i + 1;
            while (j < n && text[j] !== delimiter && text[j] !== "\n" && text[j] !== "\r") j++;
            cell += text.slice(i, j);
            i = j;
        }
    }
    if (cell !== "" || row.length > 0) {
        row.push(cell);
        rows.push(row);
    }
    return rows;
}

/** The most cells in any row. */
export function columnCount(rows: readonly string[][]): number {
    let most = 0;
    for (const r of rows) if (r.length > most) most = r.length;
    return most;
}

/** Rows read to size the columns. */
const WIDTH_ROWS = 500;

/** Each column's width in characters: its widest cell among the first rows, between `min` and `max`. */
export function columnWidths(rows: readonly string[][], columns: number, min = 4, max = 40): number[] {
    const widths = new Array<number>(columns).fill(min);
    for (let r = 0; r < Math.min(rows.length, WIDTH_ROWS); r++) {
        const row = rows[r];
        for (let c = 0; c < row.length; c++) widths[c] = Math.min(max, Math.max(widths[c], row[c].length));
    }
    return widths;
}
