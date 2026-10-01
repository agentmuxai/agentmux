// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which part of a file a Read, Edit or Write covers: shown on the tool row
 * before the path (`Read 33:334 src/a.ts`), and again above the preview
 * (docs/analysis/ANALYSIS_READ_TOOL_PREVIEW_2026_10_01.md §3).
 *
 * Pure, like the rest of tool-meta: no JSX, no Solid.
 *
 * READ, three sources, most reliable first:
 *  1. `result.range`: the CLI's own `startLine` / `numLines` / `totalLines`
 *     (kept from its structured `tool_use_result` by the translator). The only
 *     source that knows the file's total length and whether the CLI cut the
 *     read short at its token cap.
 *  2. The result text: Claude Code's `<N>\t<code>` line-number gutter, and the
 *     "showing lines A-B of N total" note it appends when it truncates. Covers
 *     results recorded before (1) was kept.
 *  3. The call's own `offset` / `limit` (and `pages` for a PDF). The only source
 *     while the read is still running; wrong once the file turns out shorter
 *     than asked, which is why (1) and (2) outrank it.
 *
 * EDIT: the changed lines, in the file as it is after the edit, from the
 * CLI's `structuredPatch` (the translator keeps only the numbers: `result.patch`).
 * Known only once the result is in; before that the call has just the old and
 * new strings, and the pane doesn't have the file.
 *
 * WRITE: always the whole file (the tool has no partial form), so `1:N` from the
 * content being written, known while it runs. An overwrite also reports which
 * lines differ from the file it replaced.
 */

import type { ToolNode } from "../types";
import { toolNameOf } from "./tool-descriptors";

/** One changed stretch of an edited file, in the file as it is after the edit. */
export interface PatchSpan {
    start: number;
    end: number;
    added: number;
    removed: number;
}

export interface FileRange {
    action: "read" | "edit" | "write";
    unit: "lines" | "pages";
    /** First line (or page), 1-based. */
    start: number;
    /** Last line (or page); null while open-ended (an offset with no limit, before the result). */
    end: number | null;
    /** The file's line count, when known. */
    total: number | null;
    /** The CLI cut the read short at its token cap. */
    capped: boolean;
    /** The read returned no lines: it started past the end of the file. */
    empty: boolean;
    /** An edit's (or an overwrite's) changed stretches; more than one for several hunks. */
    spans: PatchSpan[];
    /** A Write that created the file, as opposed to replacing one. */
    created: boolean;
    source: "result" | "gutter" | "params";
}

const READ_NAMES = new Set(["Read", "read", "read_file"]);
const WRITE_NAMES = new Set(["Write", "write", "write_file"]);
const EDIT_NAMES = new Set(["Edit", "edit", "str_replace_editor", "multiedit"]);

type Rec = Record<string, unknown>;
const rec = (v: unknown): Rec | null => (v && typeof v === "object" ? (v as Rec) : null);

const toInt = (v: unknown): number | null => {
    const n = typeof v === "string" && v.trim() !== "" ? Number(v) : v;
    return typeof n === "number" && Number.isFinite(n) ? Math.trunc(n) : null;
};

const NUMBERED_RE = /^\s*(\d+)\t/;
/** The note Claude Code appends to a token-capped read. */
const PARTIAL_RE = /showing lines (\d+)-(\d+) of (\d+) total/;

const base = (action: FileRange["action"]): Pick<FileRange, "action" | "unit" | "capped" | "empty" | "spans" | "created"> => ({
    action,
    unit: "lines",
    capped: false,
    empty: false,
    spans: [],
    created: false,
});

// ── Read ────────────────────────────────────────────────────────────────

function readFromMeta(result: Rec | null): FileRange | null {
    const m = rec(result?.range);
    const start = toInt(m?.startLine);
    const count = toInt(m?.numLines);
    if (start == null || count == null || count < 0) return null;
    return {
        ...base("read"),
        start,
        // `numLines: 0` is a read past the end of the file: say so, rather than
        // falling back to the range the call asked for (ReAgent on #4159).
        end: count === 0 ? null : start + count - 1,
        total: toInt(m?.totalLines),
        capped: m?.truncatedByTokenCap === true,
        empty: count === 0,
        source: "result",
    };
}

function readFromText(result: Rec | null): FileRange | null {
    const content = result?.content;
    if (typeof content !== "string" || content === "") return null;
    const lines = content.split("\n");
    let start: number | null = null;
    let end: number | null = null;
    for (const line of lines) {
        const m = NUMBERED_RE.exec(line);
        if (m) {
            start = Number(m[1]);
            break;
        }
    }
    for (let i = lines.length - 1; i >= 0; i--) {
        const m = NUMBERED_RE.exec(lines[i]);
        if (m) {
            end = Number(m[1]);
            break;
        }
    }
    if (start == null || end == null || end < start) return null;
    const partial = PARTIAL_RE.exec(content);
    return {
        ...base("read"),
        start,
        end,
        total: partial ? Number(partial[3]) : null,
        capped: partial != null,
        source: "gutter",
    };
}

function readFromParams(name: string, rawParams: unknown): FileRange | null {
    const params = rec(rawParams);
    if (!params) return null;
    // A PDF: `pages` is "3" or "1-5".
    if (typeof params.pages === "string" && params.pages.trim() !== "") {
        const m = /^\s*(\d+)\s*(?:-\s*(\d+))?\s*$/.exec(params.pages);
        if (m) {
            return {
                ...base("read"),
                unit: "pages",
                start: Number(m[1]),
                end: m[2] ? Number(m[2]) : Number(m[1]),
                total: null,
                source: "params",
            };
        }
    }
    const startLine = toInt(params.start_line);
    const endLine = toInt(params.end_line);
    if (startLine != null || endLine != null) {
        return { ...base("read"), start: startLine ?? 1, end: endLine, total: null, source: "params" };
    }
    // `offset` / `limit` are Claude's, and 1-based there (its structured result
    // reports `startLine: 1` for a default read). Another provider's `offset`
    // may be 0-based, so it is not guessed at.
    if (name !== "Read") return null;
    const offset = toInt(params.offset);
    const limit = toInt(params.limit);
    if (offset == null && limit == null) return null;
    const start = Math.max(1, offset ?? 1);
    return {
        ...base("read"),
        start,
        end: limit != null && limit > 0 ? start + limit - 1 : null,
        total: null,
        source: "params",
    };
}

// ── Edit and Write ──────────────────────────────────────────────────────

/** The changed stretches the translator kept from `structuredPatch`, validated. */
function patchSpans(result: Rec | null): PatchSpan[] {
    const raw = result?.patch;
    if (!Array.isArray(raw)) return [];
    const spans: PatchSpan[] = [];
    for (const item of raw) {
        const s = rec(item);
        const start = toInt(s?.start);
        const end = toInt(s?.end);
        if (start == null || end == null || end < start) continue;
        spans.push({ start, end, added: toInt(s?.added) ?? 0, removed: toInt(s?.removed) ?? 0 });
    }
    return spans;
}

function editRange(result: Rec | null): FileRange | null {
    const spans = patchSpans(result);
    if (spans.length === 0) return null;
    return {
        ...base("edit"),
        start: spans[0].start,
        end: spans[spans.length - 1].end,
        total: null,
        spans,
        source: "result",
    };
}

/** Lines in `text`, the way an editor counts them: a trailing newline doesn't start another. */
function countLines(text: string): number {
    if (text === "") return 0;
    const n = text.split("\n").length;
    return text.endsWith("\n") ? n - 1 : n;
}

function writeRange(node: Pick<ToolNode, "params" | "result">): FileRange | null {
    const params = rec(node.params);
    const content = params?.content;
    if (typeof content !== "string") return null;
    const total = countLines(content);
    if (total === 0) return null;
    const result = rec(node.result);
    return {
        ...base("write"),
        start: 1,
        end: total,
        total,
        spans: patchSpans(result),
        created: result?.writeKind === "create",
        source: "params",
    };
}

/** The range a Read, Edit or Write covers, or null when nothing is known yet. */
export function fileRangeOf(node: Pick<ToolNode, "tool" | "toolName" | "params" | "result">): FileRange | null {
    const name = toolNameOf(node);
    if (READ_NAMES.has(name)) {
        const result = rec(node.result);
        return readFromMeta(result) ?? readFromText(result) ?? readFromParams(name, node.params);
    }
    if (EDIT_NAMES.has(name)) return editRange(rec(node.result));
    if (WRITE_NAMES.has(name)) return writeRange(node);
    return null;
}

// ── Formatting ──────────────────────────────────────────────────────────

const spanText = (s: { start: number; end: number }): string => `${s.start}:${s.end}`;

/** Header chip: `120:179`, `120:179 of 456`, `1:214`, `5:11, 40:46`, `pages 1–5`. First and last line, as an editor writes them. */
export function formatFileRangeShort(r: FileRange): string {
    if (r.empty) return r.total != null ? `past end (${r.total} lines)` : "0 lines";
    if (r.unit === "pages") return r.end != null && r.end !== r.start ? `pages ${r.start}–${r.end}` : `page ${r.start}`;
    if (r.action === "edit") {
        if (r.spans.length <= 2) return r.spans.map(spanText).join(", ");
        return `${r.start}:${r.end} · ${r.spans.length} places`;
    }
    // Always first:last, even for one line (`42:42`): a lone number reads as a count.
    const span = r.end == null ? `${r.start}:` : `${r.start}:${r.end}`;
    return r.total != null && r.end !== r.total && r.action === "read" ? `${span} of ${r.total}` : span;
}

const signed = (added: number, removed: number): string => `+${added} −${removed}`;

/** Above the preview: `lines 120–179 of 456`, `changed lines 106:142 (+18 −5)`, `new file, 214 lines`. */
export function formatFileRangeLong(r: FileRange): string {
    if (r.empty) return r.total != null ? `no lines read: line ${r.start} is past the end of a ${r.total}-line file` : "no lines read";
    if (r.unit === "pages") return formatFileRangeShort(r);
    if (r.action === "edit") {
        const added = r.spans.reduce((n, s) => n + s.added, 0);
        const removed = r.spans.reduce((n, s) => n + s.removed, 0);
        const where =
            r.spans.length === 1 ? `lines ${spanText(r.spans[0])}` : `${r.spans.length} places, lines ${r.start}:${r.end}`;
        return `changed ${where} (${signed(added, removed)})`;
    }
    if (r.action === "write") {
        if (r.created) return `new file, ${r.total} lines`;
        const differs = r.spans.length > 0 ? ` · differs at ${r.spans.map(spanText).join(", ")}` : "";
        return `all ${r.total} lines${differs}`;
    }
    let text: string;
    if (r.start === 1 && r.end != null && r.total != null && r.end === r.total) {
        text = `all ${r.total} lines`;
    } else {
        const span = r.end == null ? `from line ${r.start}` : r.end === r.start ? `line ${r.start}` : `lines ${r.start}–${r.end}`;
        text = r.total != null ? `${span} of ${r.total}` : span;
    }
    return r.capped ? `${text} · cut off at the token cap` : text;
}
