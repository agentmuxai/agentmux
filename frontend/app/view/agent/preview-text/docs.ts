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
import { capChars, capLines, capRawLines, MAX_TOOL_OUTPUT_CHARS, MAX_TOOL_OUTPUT_LINES } from "./cap";
import { CODE_TAB_WIDTH, expandTabs, OUTPUT_TAB_WIDTH } from "./tabs";
import { decodeTerminal } from "./terminal";
import type { DiffMarker, PreviewDoc, PreviewLine } from "./types";

const splitLines = (text: string): string[] => text.split("\n").map((l) => (l.endsWith("\r") ? l.slice(0, -1) : l));

/** Record that the character cap cut `text`, keeping the `from` end. */
const withCut = <T extends PreviewDoc>(doc: T, text: string, from: "head" | "tail"): T =>
    text.length > MAX_TOOL_OUTPUT_CHARS ? { ...doc, truncated: from } : doc;

/** A final newline ends the last line; it doesn't start an empty one. */
function withoutFinalEmptyLine<T extends { text: string }>(lines: T[]): T[] {
    return lines.length > 1 && lines[lines.length - 1].text === "" ? lines.slice(0, -1) : lines;
}

// ── Read's line-number gutter ────────────────────────────────────────────

/** The start of a Claude Code Read line: its number (right-aligned or not),
 *  then a tab or `→`. Shared with tool-meta/file-range.ts. */
export const READ_LINE_NUMBER_RE = /^\s*(\d+)(?:\t|→)/;
// [\s\S], not `.`: a line can hold a lone \r or U+2028, which `.` won't match.
const GUTTER_RE = new RegExp(READ_LINE_NUMBER_RE.source + "([\\s\\S]*)$");

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

/**
 * File text for a Markdown preview: capped like the code view (the same
 * lines, no marker line in the text: the preview shows the code doc's
 * markers), the Read gutter taken off when `gutter`, then dedented only —
 * never narrowed, since indentation is syntax in Markdown.
 */
export function markdownBodyText(text: string, opts: { gutter: boolean }): string {
    const lines = capRawLines(splitLines(capChars(text, "head")), "head").lines;
    const split = opts.gutter ? splitGutter(lines) : null;
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
    return withCut(
        {
            kind: "code",
            lang: detectLanguage(opts.path, code.find((l) => l.trim() !== "") ?? ""),
            lines,
            ...(capped.hidden > 0 ? { hidden: { count: capped.hidden, from: "head" as const } } : {}),
        },
        text,
        "head"
    );
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
    const cut = oldStr.length > MAX_TOOL_OUTPUT_CHARS || newStr.length > MAX_TOOL_OUTPUT_CHARS;
    oldStr = capChars(oldStr, "head");
    newStr = capChars(newStr, "head");
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
    const doc = capLines({ kind: "diff" as const, lang: detectLanguage(path), lines }, "head");
    return cut ? { ...doc, truncated: "head" } : doc;
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
    return withCut(capLines({ kind: "diff", lang: detectLanguage(path), lines }, "head"), diff, "head");
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
    return withCut(capLines({ kind: "output", lines }, opts.from), text, opts.from);
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
    return withCut(
        capLines({ kind: "output", lines }, "tail"),
        stdout.length > stderr.length ? stdout : stderr,
        "tail"
    );
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
export function chunksDoc(
    chunks: readonly OutputChunk[],
    opts: { from?: "head" | "tail"; wholeLines?: ReadonlySet<OutputChunk> } = {}
): PreviewDoc {
    let cut = false;
    const raw: { text: string; kind: string }[] = [];
    let open = false;
    for (const chunk of chunks) {
        if (chunk.content === "") continue; // nothing to add; would open an empty line
        // A chunk that stands as whole lines (a collapsed spinner frame, which
        // comes back trimmed): it starts on a line of its own and ends it.
        const whole = opts.wholeLines?.has(chunk) ?? false;
        // An open line continues only from the same stream: stdout "out" then
        // stderr "err" are two lines, as in the finished output.
        if (whole || (open && raw[raw.length - 1].kind !== chunk.kind)) open = false;
        const parts = chunk.content.split("\n");
        parts.forEach((part, i) => {
            if (i === 0 && open) raw[raw.length - 1].text += part;
            else raw.push({ text: part, kind: chunk.kind });
        });
        open = !whole && !chunk.content.endsWith("\n");
        if (chunk.content.endsWith("\n")) raw.pop(); // the empty piece after a final newline
    }
    // One decode over the joined lines, so colour carries from one line to the
    // next as it does in the finished output. No joined line holds a "\n", so
    // the decoded lines match `raw` one to one.
    // Each run of one stream is decoded on its own, so colour doesn't carry
    // from stdout into stderr (as commandDoc decodes them apart).
    const lines: PreviewLine[] = [];
    for (let i = 0; i < raw.length;) {
        let j = i;
        while (j < raw.length && raw[j].kind === raw[i].kind) j++;
        const run = raw.slice(i, j).map(({ text }) => {
            if (text.length > MAX_TOOL_OUTPUT_CHARS) cut = true;
            return capChars(text, "tail");
        });
        const kind = raw[i].kind;
        for (const line of decodeTerminal(run.join("\n"), OUTPUT_TAB_WIDTH)) {
            lines.push(kind === "stderr" || kind === "system" ? { ...line, stream: kind } : line);
        }
        i = j;
    }
    const doc = capLines({ kind: "output" as const, lines }, opts.from ?? "tail");
    return cut ? { ...doc, truncated: "tail" } : doc;
}

const countNewlines = (s: string): number => {
    let n = 0;
    for (let i = s.indexOf("\n"); i !== -1; i = s.indexOf("\n", i + 1)) n++;
    return n;
};

/**
 * A window over a long, append-only chunk stream, in joined lines (what
 * {@link chunksDoc} shows), so the work per update is the window's, not the
 * whole stream's:
 *
 * - `total`: the stream's joined line count, kept up to date by counting only
 *   the chunks added since the last call (a new stream, or one that shrank,
 *   starts over), with the same joining rules as `chunksDoc`.
 * - `chunks`: the tail of the stream holding at least `maxLines` line ends,
 *   found by walking back from the end. Its first chunk can start mid-line;
 *   `chunksDoc`'s own cap then drops that partial line. A first chunk too big
 *   to process on every update is cut to its last lines (a copy).
 * - `whole`: the window's chunks that stand as whole lines (by `isWhole`,
 *   the copy included), for `chunksDoc`'s `wholeLines`.
 */
export function createChunkWindow(maxLines: number = MAX_TOOL_OUTPUT_LINES) {
    let total = 0;
    let open = false;
    let openKind = "";
    let counted = 0;
    let anchor: OutputChunk | undefined;
    return function window(
        stream: readonly OutputChunk[],
        isWhole: (c: OutputChunk) => boolean
    ): { chunks: OutputChunk[]; total: number; whole: Set<OutputChunk> } {
        if (stream.length < counted || stream[0] !== anchor) {
            total = 0;
            open = false;
            counted = 0;
            anchor = stream[0];
        }
        for (; counted < stream.length; counted++) {
            const c = stream[counted];
            if (c.content === "") continue;
            const whole = isWhole(c);
            const ends = c.content.endsWith("\n");
            const k = countNewlines(c.content);
            const joins = open && !whole && c.kind === openKind; // as chunksDoc joins
            total += (joins ? k : k + 1) - (ends ? 1 : 0);
            open = !whole && !ends;
            openKind = c.kind;
        }
        let start = stream.length;
        let lineEnds = 0;
        let chars = 0;
        while (start > 0 && lineEnds <= maxLines && chars <= MAX_TOOL_OUTPUT_CHARS) {
            const c = stream[--start];
            const next = stream[start + 1];
            // A line ends at a newline, a whole-line chunk, or where the stream
            // changes (chunksDoc starts a new line there).
            const changes = next !== undefined && next.kind !== c.kind && !c.content.endsWith("\n");
            lineEnds += countNewlines(c.content) + (isWhole(c) || changes ? 1 : 0);
            chars += c.content.length;
        }
        const chunks = stream.slice(start);
        const whole = new Set(chunks.filter(isWhole));
        const first = chunks[0];
        if (first && (first.content.length > MAX_TOOL_OUTPUT_CHARS || countNewlines(first.content) > maxLines + 1)) {
            // Keep its last maxLines + 1 lines; the extra one may be partial.
            let cut = first.content.length;
            for (let n = 0; n <= maxLines + 1 && cut > 0; n++) cut = first.content.lastIndexOf("\n", cut - 1);
            const copy = { ...first, content: capChars(first.content.slice(Math.max(0, cut + 1)), "tail") };
            if (whole.delete(first)) whole.add(copy);
            chunks[0] = copy;
        }
        return { chunks, total, whole };
    };
}

// ── Structured data and prose ────────────────────────────────────────────

/** A structured result, pretty-printed. */
export function jsonDoc(value: unknown): PreviewDoc {
    const text = value === undefined ? "" : (JSON.stringify(value, null, 2) ?? String(value));
    return withCut(
        capLines({ kind: "json", lines: splitLines(capChars(text, "head")).map((t) => ({ text: t })) }, "head"),
        text,
        "head"
    );
}

/** A control character as its Unicode control picture (␛ for ESC, ␍ for CR,
 *  ␡ for DEL), so it shows instead of acting. */
const controlPicture = (ch: string): string => {
    const code = ch.charCodeAt(0);
    return code === 0x7f ? "\u2421" : String.fromCharCode(0x2400 + code);
};

/**
 * Text shown exactly as it was received (a jekt's raw payload): split into
 * lines and nothing else. No terminal decoding, so a `\r` redraw, an erase
 * code or a backspace can't hide anything; every control character but the
 * tab shows as its control picture, and tabs are expanded.
 */
export function rawDoc(text: string): PreviewDoc {
    const lines = capChars(text, "head")
        .split("\n")
        .map((l) => ({ text: expandTabs(l.replace(/[\x00-\x08\x0b-\x1f\x7f]/g, controlPicture), OUTPUT_TAB_WIDTH) }));
    return withCut(capLines({ kind: "output", lines }, "head"), text, "head");
}

/** Prose (a message body): tabs expanded, nothing else changed. */
export function proseDoc(text: string): PreviewDoc {
    return withCut(
        capLines(
            {
                kind: "prose",
                lines: splitLines(capChars(text, "head")).map((t) => ({ text: expandTabs(t, OUTPUT_TAB_WIDTH) })),
            },
            "head"
        ),
        text,
        "head"
    );
}
