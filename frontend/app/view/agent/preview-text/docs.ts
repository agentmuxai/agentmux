// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Builders: raw tool output in, {@link PreviewDoc} out, one per kind of
 * content (REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md §5.1). Each runs the
 * same fixed steps — cap characters, decode, split, take structure off as data,
 * expand tabs, dedent and narrow, cap lines — so a preview's text is clean
 * whichever tool produced it.
 */

import { formatCodePreview, formatDiffSides, stripCommonIndent } from "../components/dedent";
import { detectLanguage } from "../components/detectLanguage";
import { capChars, capLines, capRawLines } from "./cap";
import { CODE_TAB_WIDTH, expandTabs, OUTPUT_TAB_WIDTH } from "./tabs";
import { decodeTerminal } from "./terminal";
import type { DiffMarker, PreviewDoc, PreviewLine } from "./types";

const splitLines = (text: string): string[] => text.split("\n").map((l) => (l.endsWith("\r") ? l.slice(0, -1) : l));

/** A final newline ends the last line; it doesn't start an empty one. */
function withoutFinalEmptyLine<T extends { text: string }>(lines: T[]): T[] {
    return lines.length > 1 && lines[lines.length - 1].text === "" ? lines.slice(0, -1) : lines;
}

// ── Read's line-number gutter ────────────────────────────────────────────

/** The start of a Claude Code Read line: its number (right-aligned or not),
 *  then a tab or `→`. Shared with tool-meta/file-range.ts. */
export const READ_LINE_NUMBER_RE = /^\s*(\d+)(?:\t|→)/;
const GUTTER_RE = new RegExp(READ_LINE_NUMBER_RE.source + "(.*)$");

/** Split a Read body into line numbers and code, when every line that has
 *  any text is numbered (an empty line may not be); null otherwise. */
export function splitGutter(lines: readonly string[]): { numbers: (number | undefined)[]; code: string[] } | null {
    const numbers: (number | undefined)[] = [];
    const code: string[] = [];
    let numbered = 0;
    for (const line of lines) {
        const m = GUTTER_RE.exec(line);
        if (m) {
            numbers.push(Number(m[1]));
            code.push(m[2]);
            numbered++;
        } else if (line === "") {
            numbers.push(undefined);
            code.push("");
        } else {
            return null;
        }
    }
    return numbered > 0 ? { numbers, code } : null;
}

/** A Read body without its gutter, dedented: what a Markdown preview renders. */
export function readBodyText(text: string): string {
    const lines = capRawLines(splitLines(capChars(text, "head")), "head").lines;
    const split = splitGutter(lines);
    return stripCommonIndent((split ? split.code : lines).join("\n"));
}

// ── Code (Read, Write) ───────────────────────────────────────────────────

/**
 * File content as shown in a Read or Write preview: the gutter (for a Read)
 * becomes line numbers, tabs become {@link CODE_TAB_WIDTH} spaces from the
 * code's own first column, then the common indent is removed and the rest
 * narrowed to 2 columns a level. The language is detected from the path and
 * the first line of code (not of the gutter, so a shebang is seen).
 */
export function codeDoc(text: string, opts: { path: string; gutter?: boolean }): PreviewDoc {
    const capped = capRawLines(splitLines(capChars(text, "head")), "head");
    const split = opts.gutter ? splitGutter(capped.lines) : null;
    const code = (split ? split.code : capped.lines).map((l) => expandTabs(l, CODE_TAB_WIDTH));
    const formatted = splitLines(formatCodePreview(code.join("\n")));
    const lines: PreviewLine[] = formatted.map((t, i) =>
        split?.numbers[i] != null ? { text: t, number: split.numbers[i] } : { text: t }
    );
    return {
        kind: "code",
        lang: detectLanguage(opts.path, code.find((l) => l.trim() !== "") ?? ""),
        lines,
        ...(capped.hidden > 0 ? { hidden: { count: capped.hidden, from: "head" as const } } : {}),
    };
}

// ── Diffs (Edit) ─────────────────────────────────────────────────────────

/** Line-level LCS diff bounded to this many lines a side; past it, a delete
 *  block then an add block. */
const LCS_LINE_LIMIT = 300;

function lcsEdits(a: string[], b: string[]): [DiffMarker, string][] {
    const m = a.length;
    const n = b.length;
    const dp = Array.from({ length: m + 1 }, () => new Uint16Array(n + 1));
    for (let i = 1; i <= m; i++) {
        for (let j = 1; j <= n; j++) {
            dp[i][j] = a[i - 1] === b[j - 1] ? dp[i - 1][j - 1] + 1 : Math.max(dp[i - 1][j], dp[i][j - 1]);
        }
    }
    const edits: [DiffMarker, string][] = [];
    let i = m;
    let j = n;
    while (i > 0 || j > 0) {
        if (i > 0 && j > 0 && a[i - 1] === b[j - 1]) {
            edits.push([" ", a[--i]]);
            j--;
        } else if (j > 0 && (i === 0 || dp[i][j - 1] >= dp[i - 1][j])) {
            edits.push(["+", b[--j]]);
        } else {
            edits.push(["-", a[--i]]);
        }
    }
    return edits.reverse();
}

/**
 * An Edit's old and new strings as a diff: tabs expanded on both sides, one
 * shared dedent and indent unit across both (so neither side shifts on its
 * own), then a line diff under one `@@` header.
 */
export function diffDocFromSides(oldStr: string, newStr: string, path: string): PreviewDoc {
    const expand = (s: string) =>
        splitLines(s)
            .map((l) => expandTabs(l, CODE_TAB_WIDTH))
            .join("\n");
    const sides = formatDiffSides(expand(oldStr), expand(newStr));
    const a = splitLines(sides.oldStr);
    const b = splitLines(sides.newStr);
    const edits: [DiffMarker, string][] =
        a.length <= LCS_LINE_LIMIT && b.length <= LCS_LINE_LIMIT
            ? lcsEdits(a, b)
            : [...a.map((l): [DiffMarker, string] => ["-", l]), ...b.map((l): [DiffMarker, string] => ["+", l])];
    const oldCount = edits.filter(([op]) => op !== "+").length;
    const newCount = edits.filter(([op]) => op !== "-").length;
    const lines: PreviewLine[] = [
        { marker: "@@", text: `@@ -1,${oldCount} +1,${newCount} @@` },
        ...edits.map(([marker, text]) => ({ marker, text })),
    ];
    return capLines({ kind: "diff", lang: detectLanguage(path), lines }, "head");
}

/** A unified diff the tool returned, one line per diff line, tabs expanded
 *  from the start of the code (after the marker). */
export function diffDocFromUnified(diff: string, path: string): PreviewDoc {
    const lines: PreviewLine[] = withoutFinalEmptyLine(
        splitLines(capChars(diff, "head")).map((raw): PreviewLine => {
            if (raw.startsWith("@@")) return { marker: "@@", text: raw };
            const marker: DiffMarker = raw.startsWith("+") ? "+" : raw.startsWith("-") ? "-" : " ";
            const body = raw.startsWith("+") || raw.startsWith("-") || raw.startsWith(" ") ? raw.slice(1) : raw;
            return { marker, text: expandTabs(body, CODE_TAB_WIDTH) };
        })
    );
    return capLines({ kind: "diff", lang: detectLanguage(path), lines }, "head");
}

// ── Command and tool output ──────────────────────────────────────────────

/**
 * Command or tool output: decoded as a terminal shows it, tabs at
 * {@link OUTPUT_TAB_WIDTH}. `from` is the end that matters: "tail" for a
 * command (the latest), "head" for a list read top-down (search results).
 */
export function outputDoc(text: string, opts: { from: "head" | "tail"; stream?: "stderr" }): PreviewDoc {
    let lines = withoutFinalEmptyLine(decodeTerminal(capChars(text, opts.from), OUTPUT_TAB_WIDTH));
    if (opts.stream) lines = lines.map((l) => ({ ...l, stream: opts.stream }));
    return capLines({ kind: "output", lines }, opts.from);
}

/** A finished command: its output, then its error output, as one preview. */
export function commandDoc(stdout: string, stderr: string): PreviewDoc {
    const out = stdout ? decodeTerminal(capChars(stdout, "tail"), OUTPUT_TAB_WIDTH) : [];
    const err = stderr
        ? decodeTerminal(capChars(stderr, "tail"), OUTPUT_TAB_WIDTH).map((l) => ({ ...l, stream: "stderr" as const }))
        : [];
    const lines = [...withoutFinalEmptyLine(out), ...withoutFinalEmptyLine(err)].filter(
        (l, i, all) => all.length > 1 || l.text !== ""
    );
    return capLines({ kind: "output", lines }, "tail");
}

/** A chunk of streamed output, as the reducer stores it. */
export interface OutputChunk {
    kind: string;
    content: string;
}

/**
 * Streamed output as one preview. Chunks are cut wherever the stream flushed,
 * often mid-line, so they are joined back into lines here; a line takes the
 * stream of the chunk that started it. The finished result of the same output
 * then renders identically.
 */
export function chunksDoc(chunks: readonly OutputChunk[], opts: { from?: "head" | "tail" } = {}): PreviewDoc {
    const raw: { text: string; kind: string }[] = [];
    let open = false;
    for (const chunk of chunks) {
        if (chunk.content === "") continue; // nothing to add; would open an empty line
        const parts = chunk.content.split("\n");
        parts.forEach((part, i) => {
            if (i === 0 && open) raw[raw.length - 1].text += part;
            else raw.push({ text: part, kind: chunk.kind });
        });
        open = !chunk.content.endsWith("\n");
        if (!open) raw.pop(); // the empty piece after a final newline
    }
    const lines: PreviewLine[] = raw.map(({ text, kind }) => {
        const line = decodeTerminal(capChars(text, "tail"), OUTPUT_TAB_WIDTH).at(-1) ?? { text: "" };
        return kind === "stderr" || kind === "system" ? { ...line, stream: kind } : line;
    });
    return capLines({ kind: "output", lines }, opts.from ?? "tail");
}

// ── Structured data and prose ────────────────────────────────────────────

/** A structured result, pretty-printed. */
export function jsonDoc(value: unknown): PreviewDoc {
    const text = value === undefined ? "" : (JSON.stringify(value, null, 2) ?? String(value));
    return capLines({ kind: "json", lines: splitLines(capChars(text, "head")).map((t) => ({ text: t })) }, "head");
}

/** Prose (a message body): tabs expanded, nothing else changed. */
export function proseDoc(text: string): PreviewDoc {
    return capLines(
        {
            kind: "prose",
            lines: splitLines(capChars(text, "head")).map((t) => ({ text: expandTabs(t, OUTPUT_TAB_WIDTH) })),
        },
        "head"
    );
}
