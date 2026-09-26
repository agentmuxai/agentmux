// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { grepResultCount } from "./grep-result";

// Claude Code's real Grep result text, one per output_mode, captured from
// live Grep calls on 2026-09-26 (Opaz P1 on #3877: the default mode is
// files_with_matches, not content).
describe("grepResultCount", () => {
    it("files_with_matches (the default): the Found N files header", () => {
        const text = "Found 2 files\nagentmux\a\ToolBlock.tsx\nagentmux\a\tool-header.ts";
        expect(grepResultCount(text)).toEqual({ n: 2, noun: "file" });
        expect(grepResultCount("Found 1 file\n/a.ts")).toEqual({ n: 1, noun: "file" });
    });

    it("content: one match per path:line:text line; ignores separators and the pagination notice", () => {
        const text = ["a.ts:54:x", "a.ts:61:x", "--", "b.ts:52:x", "", "[Showing results with pagination = limit: 60]"].join("\n");
        expect(grepResultCount(text)).toEqual({ n: 3, noun: "match" });
    });

    it("count: the total from the trailer, not the per-file lines", () => {
        const text = "b.tsx:2\na.ts:3\n\nFound 5 total occurrences across 2 files.";
        expect(grepResultCount(text)).toEqual({ n: 5, noun: "match" });
    });

    it("no results in any mode reads as zero, not one", () => {
        expect(grepResultCount("No files found")).toEqual({ n: 0, noun: "file" });
        expect(grepResultCount("No matches found")).toEqual({ n: 0, noun: "match" });
        expect(grepResultCount("No matches found\n\nFound 0 total occurrences across 0 files.")).toEqual({ n: 0, noun: "match" });
    });
});
