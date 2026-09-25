// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Same cell cap as the backend's MAX_DIFF_CELLS (native_memory_handlers.rs). */
const MAX_DIFF_CELLS = 4_000_000;

/**
 * Client-side twin of the backend's `line_diff` (native_memory_handlers.rs):
 * LCS-based, one output line per input line, prefixed `"  "`, `"- "` or
 * `"+ "`. Used only for the "View change" panel of the dirty-draft banner,
 * which compares two strings that are not both recorded versions (the
 * draft's base and whatever is saved now), so the server's version-id diff
 * can't answer it. Output format matches so one renderer serves both.
 */
export function lineDiff(from: string, to: string): string {
    const a = splitLines(from);
    const b = splitLines(to);
    const n = a.length;
    const m = b.length;
    if (n * m > MAX_DIFF_CELLS) {
        return `(diff omitted: ${n} x ${m} lines exceeds the ${MAX_DIFF_CELLS}-cell comparison cap)`;
    }
    const cols = m + 1;
    const lcs = new Uint32Array((n + 1) * cols);
    for (let i = n - 1; i >= 0; i--) {
        for (let j = m - 1; j >= 0; j--) {
            lcs[i * cols + j] =
                a[i] === b[j] ? lcs[(i + 1) * cols + j + 1] + 1 : Math.max(lcs[(i + 1) * cols + j], lcs[i * cols + j + 1]);
        }
    }
    const out: string[] = [];
    let i = 0;
    let j = 0;
    while (i < n && j < m) {
        if (a[i] === b[j]) {
            out.push(`  ${a[i]}`);
            i++;
            j++;
        } else if (lcs[(i + 1) * cols + j] >= lcs[i * cols + j + 1]) {
            out.push(`- ${a[i++]}`);
        } else {
            out.push(`+ ${b[j++]}`);
        }
    }
    while (i < n) out.push(`- ${a[i++]}`);
    while (j < m) out.push(`+ ${b[j++]}`);
    return out.map((l) => `${l}\n`).join("");
}

/** Rust's `str::lines()`: split on \n (dropping a trailing \r), no final
 *  empty element for a trailing newline. */
function splitLines(s: string): string[] {
    if (s === "") return [];
    const lines = s.split("\n").map((l) => (l.endsWith("\r") ? l.slice(0, -1) : l));
    if (lines[lines.length - 1] === "") lines.pop();
    return lines;
}

/** CSS class for one line of either diff (server or client). */
export function diffLineClass(line: string): string {
    if (line.startsWith("+ ")) return "native-memory-diff-line is-added";
    if (line.startsWith("- ")) return "native-memory-diff-line is-removed";
    return "native-memory-diff-line";
}

/** Split diff text into renderable lines, dropping the one empty element a
 *  trailing newline leaves behind. */
export function diffLines(text: string): string[] {
    const lines = text.split("\n");
    if (lines[lines.length - 1] === "") lines.pop();
    return lines;
}
