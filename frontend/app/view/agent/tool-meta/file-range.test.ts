// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { ToolNode } from "../types";
import { formatFileRangeLong, formatFileRangeShort, fileRangeOf } from "./file-range";

const node = (params: Record<string, unknown>, result?: unknown, toolName = "Read"): ToolNode =>
    ({ type: "tool", id: "t", tool: "Read", toolName, params, result, status: "success", collapsed: true, summary: "" }) as ToolNode;

describe("fileRangeOf: Read", () => {
    it("uses the CLI's own numbers first, with the file's total", () => {
        const r = fileRangeOf(
            node({ file_path: "a.ts", offset: 5, limit: 10 }, { content: "   120\tx", range: { startLine: 120, numLines: 60, totalLines: 456 } })
        )!;
        expect(r).toMatchObject({ start: 120, end: 179, total: 456, capped: false, source: "result" });
        expect(formatFileRangeShort(r)).toBe("120:179 of 456");
        expect(formatFileRangeLong(r)).toBe("lines 120–179 of 456");
    });

    it("says when the CLI cut a read short at its token cap", () => {
        const r = fileRangeOf(node({ file_path: "a.rs" }, { content: "x", range: { startLine: 1, numLines: 1082, totalLines: 1320, truncatedByTokenCap: true } }))!;
        expect(formatFileRangeShort(r)).toBe("1:1082 of 1320");
        expect(formatFileRangeLong(r)).toBe("lines 1–1082 of 1320 · cut off at the token cap");
    });

    it("names a whole-file read by its length", () => {
        const r = fileRangeOf(node({ file_path: "a.ts" }, { content: "x", range: { startLine: 1, numLines: 214, totalLines: 214 } }))!;
        expect(formatFileRangeShort(r)).toBe("1:214");
        expect(formatFileRangeLong(r)).toBe("all 214 lines");
    });

    it("falls back to the line-number gutter for results recorded without the numbers", () => {
        const r = fileRangeOf(node({ file_path: "a.ts", offset: 120, limit: 500 }, { content: "   120\tfoo\n   121\tbar\n   122\t" }))!;
        expect(r).toMatchObject({ start: 120, end: 122, total: null, source: "gutter" });
        expect(formatFileRangeShort(r)).toBe("120:122");
    });

    it("reads the total from the CLI's truncation note when there is no structured result", () => {
        const content =
            "     1\tone\n     2\ttwo\n\n<system-reminder>[Truncated: PARTIAL view — showing lines 1-2 of 1320 total (25901 tokens, cap 25000).]</system-reminder>";
        const r = fileRangeOf(node({ file_path: "a.ts" }, { content }))!;
        expect(r).toMatchObject({ start: 1, end: 2, total: 1320, capped: true, source: "gutter" });
    });

    it("uses offset and limit while the read is still running", () => {
        const r = fileRangeOf(node({ file_path: "a.ts", offset: 120, limit: 60 }))!;
        expect(r).toMatchObject({ start: 120, end: 179, source: "params" });
        expect(formatFileRangeShort(r)).toBe("120:179");
    });

    it("is open-ended for an offset with no limit, and from line 1 for a limit with no offset", () => {
        expect(formatFileRangeShort(fileRangeOf(node({ file_path: "a.ts", offset: 300 }))!)).toBe("300:");
        expect(formatFileRangeShort(fileRangeOf(node({ file_path: "a.ts", limit: 50 }))!)).toBe("1:50");
    });

    it("has nothing to say about a whole-file read that has no result yet", () => {
        expect(fileRangeOf(node({ file_path: "a.ts" }))).toBeNull();
    });

    it("reads a PDF's pages", () => {
        expect(formatFileRangeShort(fileRangeOf(node({ file_path: "a.pdf", pages: "1-5" }))!)).toBe("pages 1–5");
        expect(formatFileRangeShort(fileRangeOf(node({ file_path: "a.pdf", pages: "3" }))!)).toBe("page 3");
    });

    it("does not guess another provider's offset, but takes explicit start and end lines", () => {
        expect(fileRangeOf(node({ path: "a.ts", offset: 10, limit: 5 }, undefined, "read_file"))).toBeNull();
        const r = fileRangeOf(node({ path: "a.ts", start_line: 10, end_line: 20 }, undefined, "read_file"))!;
        expect(formatFileRangeShort(r)).toBe("10:20");
    });

    // ReAgent on #4159: `numLines: 0` is a read past the end of the file.
    it("reports a read past the end of the file as empty, not as the range the call asked for", () => {
        const r = fileRangeOf(
            node({ file_path: "a.ts", offset: 500, limit: 50 }, { content: "", range: { startLine: 500, numLines: 0, totalLines: 456 } })
        )!;
        expect(r).toMatchObject({ empty: true, total: 456, source: "result" });
        expect(formatFileRangeShort(r)).toBe("past end (456 lines)");
        expect(formatFileRangeLong(r)).toBe("no lines read: line 500 is past the end of a 456-line file");
    });

    it("says only that no lines were read when the file's length is unknown", () => {
        const r = fileRangeOf(node({ file_path: "a.ts" }, { content: "", range: { startLine: 1, numLines: 0 } }))!;
        expect(formatFileRangeShort(r)).toBe("0 lines");
        expect(formatFileRangeLong(r)).toBe("no lines read");
    });

    it("ignores other tools", () => {
        expect(fileRangeOf(node({ file_path: "a.ts", offset: 1, limit: 2 }, undefined, "Edit"))).toBeNull();
    });

    it("takes numbers sent as strings", () => {
        expect(formatFileRangeShort(fileRangeOf(node({ file_path: "a.ts", offset: "7", limit: "3" }))!)).toBe("7:9");
    });

    it("shows a one-line range as a single line", () => {
        expect(formatFileRangeShort(fileRangeOf(node({ file_path: "a.ts", offset: 42, limit: 1 }))!)).toBe("42:42");
        expect(formatFileRangeLong(fileRangeOf(node({ file_path: "a.ts", offset: 42, limit: 1 }))!)).toBe("line 42");
    });
});

const tool = (toolName: string, params: Record<string, unknown>, result?: unknown): ToolNode =>
    ({ type: "tool", id: "t", tool: "Edit", toolName, params, result, status: "success", collapsed: true, summary: "" }) as ToolNode;

describe("fileRangeOf: Edit", () => {
    it("is the changed lines in the edited file, from the patch the translator kept", () => {
        const r = fileRangeOf(
            tool("Edit", { file_path: "a.ts" }, { content: "ok", patch: [{ start: 109, end: 157, added: 18, removed: 5 }] })
        )!;
        expect(r).toMatchObject({ action: "edit", start: 109, end: 157 });
        expect(formatFileRangeShort(r)).toBe("109:157");
        expect(formatFileRangeLong(r)).toBe("changed lines 109:157 (+18 −5)");
    });

    it("lists two places, and summarizes more", () => {
        const two = fileRangeOf(
            tool("Edit", {}, { patch: [{ start: 5, end: 11, added: 3, removed: 3 }, { start: 40, end: 46, added: 2, removed: 1 }] })
        )!;
        expect(formatFileRangeShort(two)).toBe("5:11, 40:46");
        expect(formatFileRangeLong(two)).toBe("changed 2 places, lines 5:46 (+5 −4)");
        const many = fileRangeOf(
            tool("Edit", {}, { patch: [1, 2, 3].map((n) => ({ start: n * 10, end: n * 10 + 1, added: 1, removed: 0 })) })
        )!;
        expect(formatFileRangeShort(many)).toBe("10:31 · 3 places");
    });

    it("has nothing to say until the result is in, or when it carries no patch", () => {
        expect(fileRangeOf(tool("Edit", { file_path: "a.ts", old_string: "a", new_string: "b" }))).toBeNull();
        expect(fileRangeOf(tool("Edit", {}, { content: "ok" }))).toBeNull();
        expect(fileRangeOf(tool("Edit", {}, { patch: [{ start: 9, end: 3 }] }))).toBeNull();
    });
});

describe("fileRangeOf: Write", () => {
    it("is the whole file, known while the write runs", () => {
        const r = fileRangeOf(tool("Write", { file_path: "a.ts", content: "one\ntwo\nthree\n" }))!;
        expect(r).toMatchObject({ action: "write", start: 1, end: 3, total: 3 });
        expect(formatFileRangeShort(r)).toBe("1:3");
        expect(formatFileRangeLong(r)).toBe("all 3 lines");
    });

    it("says when it created the file, and where an overwrite differs", () => {
        const created = fileRangeOf(tool("Write", { content: "a\nb" }, { writeKind: "create", patch: [] }))!;
        expect(formatFileRangeLong(created)).toBe("new file, 2 lines");
        const updated = fileRangeOf(
            tool("Write", { content: "a\nb\nc\nd" }, { writeKind: "update", patch: [{ start: 2, end: 3, added: 2, removed: 1 }] })
        )!;
        expect(formatFileRangeShort(updated)).toBe("1:4");
        expect(formatFileRangeLong(updated)).toBe("all 4 lines · differs at 2:3");
    });

    it("counts lines the way an editor does, and has nothing for an empty write", () => {
        expect(fileRangeOf(tool("Write", { content: "a\n\nb" }))!.end).toBe(3);
        expect(fileRangeOf(tool("Write", { content: "" }))).toBeNull();
        expect(fileRangeOf(tool("Write", {}))).toBeNull();
    });
});
