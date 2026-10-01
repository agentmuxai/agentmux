// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { ToolNode } from "../types";
import { formatReadRangeLong, formatReadRangeShort, readRangeOf } from "./read-range";

const node = (params: Record<string, unknown>, result?: unknown, toolName = "Read"): ToolNode =>
    ({ type: "tool", id: "t", tool: "Read", toolName, params, result, status: "success", collapsed: true, summary: "" }) as ToolNode;

describe("readRangeOf", () => {
    it("uses the CLI's own numbers first, with the file's total", () => {
        const r = readRangeOf(
            node({ file_path: "a.ts", offset: 5, limit: 10 }, { content: "   120\tx", range: { startLine: 120, numLines: 60, totalLines: 456 } })
        )!;
        expect(r).toMatchObject({ start: 120, end: 179, total: 456, capped: false, source: "result" });
        expect(formatReadRangeShort(r)).toBe("L120–179 of 456");
        expect(formatReadRangeLong(r)).toBe("lines 120–179 of 456");
    });

    it("says when the CLI cut a read short at its token cap", () => {
        const r = readRangeOf(node({ file_path: "a.rs" }, { content: "x", range: { startLine: 1, numLines: 1082, totalLines: 1320, truncatedByTokenCap: true } }))!;
        expect(formatReadRangeShort(r)).toBe("L1–1082 of 1320");
        expect(formatReadRangeLong(r)).toBe("lines 1–1082 of 1320 · cut off at the token cap");
    });

    it("names a whole-file read by its length", () => {
        const r = readRangeOf(node({ file_path: "a.ts" }, { content: "x", range: { startLine: 1, numLines: 214, totalLines: 214 } }))!;
        expect(formatReadRangeShort(r)).toBe("214 lines");
        expect(formatReadRangeLong(r)).toBe("all 214 lines");
    });

    it("falls back to the line-number gutter for results recorded without the numbers", () => {
        const r = readRangeOf(node({ file_path: "a.ts", offset: 120, limit: 500 }, { content: "   120\tfoo\n   121\tbar\n   122\t" }))!;
        expect(r).toMatchObject({ start: 120, end: 122, total: null, source: "gutter" });
        expect(formatReadRangeShort(r)).toBe("L120–122");
    });

    it("reads the total from the CLI's truncation note when there is no structured result", () => {
        const content =
            "     1\tone\n     2\ttwo\n\n<system-reminder>[Truncated: PARTIAL view — showing lines 1-2 of 1320 total (25901 tokens, cap 25000).]</system-reminder>";
        const r = readRangeOf(node({ file_path: "a.ts" }, { content }))!;
        expect(r).toMatchObject({ start: 1, end: 2, total: 1320, capped: true, source: "gutter" });
    });

    it("uses offset and limit while the read is still running", () => {
        const r = readRangeOf(node({ file_path: "a.ts", offset: 120, limit: 60 }))!;
        expect(r).toMatchObject({ start: 120, end: 179, source: "params" });
        expect(formatReadRangeShort(r)).toBe("L120–179");
    });

    it("is open-ended for an offset with no limit, and from line 1 for a limit with no offset", () => {
        expect(formatReadRangeShort(readRangeOf(node({ file_path: "a.ts", offset: 300 }))!)).toBe("L300–");
        expect(formatReadRangeShort(readRangeOf(node({ file_path: "a.ts", limit: 50 }))!)).toBe("L1–50");
    });

    it("has nothing to say about a whole-file read that has no result yet", () => {
        expect(readRangeOf(node({ file_path: "a.ts" }))).toBeNull();
    });

    it("reads a PDF's pages", () => {
        expect(formatReadRangeShort(readRangeOf(node({ file_path: "a.pdf", pages: "1-5" }))!)).toBe("pages 1–5");
        expect(formatReadRangeShort(readRangeOf(node({ file_path: "a.pdf", pages: "3" }))!)).toBe("page 3");
    });

    it("does not guess another provider's offset, but takes explicit start and end lines", () => {
        expect(readRangeOf(node({ path: "a.ts", offset: 10, limit: 5 }, undefined, "read_file"))).toBeNull();
        const r = readRangeOf(node({ path: "a.ts", start_line: 10, end_line: 20 }, undefined, "read_file"))!;
        expect(formatReadRangeShort(r)).toBe("L10–20");
    });

    // ReAgent on #4159: `numLines: 0` is a read past the end of the file.
    it("reports a read past the end of the file as empty, not as the range the call asked for", () => {
        const r = readRangeOf(
            node({ file_path: "a.ts", offset: 500, limit: 50 }, { content: "", range: { startLine: 500, numLines: 0, totalLines: 456 } })
        )!;
        expect(r).toMatchObject({ empty: true, total: 456, source: "result" });
        expect(formatReadRangeShort(r)).toBe("past end (456 lines)");
        expect(formatReadRangeLong(r)).toBe("no lines read: line 500 is past the end of a 456-line file");
    });

    it("says only that no lines were read when the file's length is unknown", () => {
        const r = readRangeOf(node({ file_path: "a.ts" }, { content: "", range: { startLine: 1, numLines: 0 } }))!;
        expect(formatReadRangeShort(r)).toBe("0 lines");
        expect(formatReadRangeLong(r)).toBe("no lines read");
    });

    it("ignores other tools", () => {
        expect(readRangeOf(node({ file_path: "a.ts", offset: 1, limit: 2 }, undefined, "Edit"))).toBeNull();
    });

    it("takes numbers sent as strings", () => {
        expect(formatReadRangeShort(readRangeOf(node({ file_path: "a.ts", offset: "7", limit: "3" }))!)).toBe("L7–9");
    });

    it("shows a one-line range as a single line", () => {
        expect(formatReadRangeShort(readRangeOf(node({ file_path: "a.ts", offset: 42, limit: 1 }))!)).toBe("L42");
        expect(formatReadRangeLong(readRangeOf(node({ file_path: "a.ts", offset: 42, limit: 1 }))!)).toBe("line 42");
    });
});
