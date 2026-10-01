// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which part of a file a Read covers: shown on the tool row, and again above
 * the preview (docs/analysis/ANALYSIS_READ_TOOL_PREVIEW_2026_10_01.md §3).
 *
 * Pure, like the rest of tool-meta: no JSX, no Solid.
 *
 * Three sources, most reliable first:
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
 */

import type { ToolNode } from "../types";
import { toolNameOf } from "./tool-descriptors";

export interface ReadRange {
    unit: "lines" | "pages";
    /** First line (or page), 1-based. */
    start: number;
    /** Last line (or page); null while open-ended (an offset with no limit, before the result). */
    end: number | null;
    /** The file's line count, when the CLI said. */
    total: number | null;
    /** The CLI cut the read short at its token cap. */
    capped: boolean;
    source: "result" | "gutter" | "params";
}

const READ_NAMES = new Set(["Read", "read", "read_file"]);

type Rec = Record<string, unknown>;
const rec = (v: unknown): Rec | null => (v && typeof v === "object" ? (v as Rec) : null);

const toInt = (v: unknown): number | null => {
    const n = typeof v === "string" && v.trim() !== "" ? Number(v) : v;
    return typeof n === "number" && Number.isFinite(n) ? Math.trunc(n) : null;
};

const NUMBERED_RE = /^\s*(\d+)\t/;
/** The note Claude Code appends to a token-capped read. */
const PARTIAL_RE = /showing lines (\d+)-(\d+) of (\d+) total/;

function fromMeta(result: Rec | null): ReadRange | null {
    const m = rec(result?.range);
    const start = toInt(m?.startLine);
    const count = toInt(m?.numLines);
    if (start == null || count == null || count < 1) return null;
    return {
        unit: "lines",
        start,
        end: start + count - 1,
        total: toInt(m?.totalLines),
        capped: m?.truncatedByTokenCap === true,
        source: "result",
    };
}

function fromText(result: Rec | null): ReadRange | null {
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
        unit: "lines",
        start,
        end,
        total: partial ? Number(partial[3]) : null,
        capped: partial != null,
        source: "gutter",
    };
}

function fromParams(name: string, rawParams: unknown): ReadRange | null {
    const params = rec(rawParams);
    if (!params) return null;
    // A PDF: `pages` is "3" or "1-5".
    if (typeof params.pages === "string" && params.pages.trim() !== "") {
        const m = /^\s*(\d+)\s*(?:-\s*(\d+))?\s*$/.exec(params.pages);
        if (m) {
            return {
                unit: "pages",
                start: Number(m[1]),
                end: m[2] ? Number(m[2]) : Number(m[1]),
                total: null,
                capped: false,
                source: "params",
            };
        }
    }
    const startLine = toInt(params.start_line);
    const endLine = toInt(params.end_line);
    if (startLine != null || endLine != null) {
        return {
            unit: "lines",
            start: startLine ?? 1,
            end: endLine,
            total: null,
            capped: false,
            source: "params",
        };
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
        unit: "lines",
        start,
        end: limit != null && limit > 0 ? start + limit - 1 : null,
        total: null,
        capped: false,
        source: "params",
    };
}

/** The range a Read covers, or null when it is a Read of the whole file with no result yet. */
export function readRangeOf(node: Pick<ToolNode, "tool" | "toolName" | "params" | "result">): ReadRange | null {
    const name = toolNameOf(node);
    if (!READ_NAMES.has(name)) return null;
    const result = rec(node.result);
    return fromMeta(result) ?? fromText(result) ?? fromParams(name, node.params);
}

/** Header chip: `L120–179`, `L120–179 of 456`, `214 lines`, `pages 1–5`. */
export function formatReadRangeShort(r: ReadRange): string {
    if (r.unit === "pages") return r.end != null && r.end !== r.start ? `pages ${r.start}–${r.end}` : `page ${r.start}`;
    if (r.start === 1 && r.end != null && r.total != null && r.end === r.total) return `${r.total} lines`;
    const span = r.end == null ? `L${r.start}–` : r.end === r.start ? `L${r.start}` : `L${r.start}–${r.end}`;
    return r.total != null && r.end !== r.total ? `${span} of ${r.total}` : span;
}

/** Above the preview: `lines 120–179 of 456`, with the cap noted. */
export function formatReadRangeLong(r: ReadRange): string {
    if (r.unit === "pages") return formatReadRangeShort(r);
    let text: string;
    if (r.start === 1 && r.end != null && r.total != null && r.end === r.total) {
        text = `all ${r.total} lines`;
    } else {
        const span = r.end == null ? `from line ${r.start}` : r.end === r.start ? `line ${r.start}` : `lines ${r.start}–${r.end}`;
        text = r.total != null ? `${span} of ${r.total}` : span;
    }
    return r.capped ? `${text} · cut off at the token cap` : text;
}
