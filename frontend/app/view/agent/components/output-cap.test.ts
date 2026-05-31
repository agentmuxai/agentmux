// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { capChunksByLines, capText, MAX_TOOL_OUTPUT_LINES } from "./output-cap";

describe("capText", () => {
    it("returns unchanged when under budget", () => {
        expect(capText("a\nb\nc", 10)).toEqual({ text: "a\nb\nc", hiddenLines: 0 });
    });

    it("treats exactly-at-budget as not capped", () => {
        expect(capText("a\nb", 2).hiddenLines).toBe(0);
    });

    it("keeps the tail by default", () => {
        const r = capText("a\nb\nc\nd", 2);
        expect(r.text).toBe("c\nd");
        expect(r.hiddenLines).toBe(2);
    });

    it("keeps the head when asked", () => {
        const r = capText("a\nb\nc\nd", 2, "head");
        expect(r.text).toBe("a\nb");
        expect(r.hiddenLines).toBe(2);
    });

    it("handles empty input", () => {
        expect(capText("", 5)).toEqual({ text: "", hiddenLines: 0 });
    });

    it("defaults to the module line budget", () => {
        const big = Array.from({ length: MAX_TOOL_OUTPUT_LINES + 50 }, (_, i) => `l${i}`).join("\n");
        const r = capText(big);
        expect(r.hiddenLines).toBe(50);
        expect(r.text.split("\n").length).toBe(MAX_TOOL_OUTPUT_LINES);
    });
});

describe("capChunksByLines", () => {
    const C = (content: string) => ({ content });

    it("returns the same array reference when under budget", () => {
        const chunks = [C("a"), C("b\nc")];
        const r = capChunksByLines(chunks, 10);
        expect(r.hiddenChunks).toBe(0);
        expect(r.chunks).toBe(chunks);
    });

    it("keeps the tail whose cumulative lines reach the budget", () => {
        const chunks = [C("a"), C("b"), C("c"), C("d")]; // 1 line each
        const r = capChunksByLines(chunks, 2);
        expect(r.chunks.map((c) => c.content)).toEqual(["c", "d"]);
        expect(r.hiddenChunks).toBe(2);
    });

    it("counts multi-line chunks against the budget", () => {
        const chunks = [C("a"), C("b\nc"), C("d\ne")]; // 1, 2, 2 lines
        const r = capChunksByLines(chunks, 3);
        // tail walk: "d\ne"(2) → budget 1; "b\nc"(2) → budget -1, stop, keep both
        expect(r.chunks.map((c) => c.content)).toEqual(["b\nc", "d\ne"]);
        expect(r.hiddenChunks).toBe(1);
    });

    it("keeps a single oversized tail chunk whole", () => {
        const huge = Array.from({ length: 5000 }, (_, i) => `x${i}`).join("\n");
        const chunks = [C("a"), C(huge)];
        const r = capChunksByLines(chunks, 1000);
        expect(r.chunks.map((c) => c.content)).toEqual([huge]);
        expect(r.hiddenChunks).toBe(1);
    });

    it("retains object identity for kept chunks", () => {
        const chunks = [C("a"), C("b"), C("c")];
        const r = capChunksByLines(chunks, 1);
        expect(r.chunks[r.chunks.length - 1]).toBe(chunks[chunks.length - 1]);
    });

    it("handles an empty list", () => {
        expect(capChunksByLines([], 10)).toEqual({ chunks: [], hiddenChunks: 0 });
    });
});
