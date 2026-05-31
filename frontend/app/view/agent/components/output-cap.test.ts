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

    it("ignores a trailing newline when counting against the budget", () => {
        const exact = Array.from({ length: MAX_TOOL_OUTPUT_LINES }, (_, i) => `l${i}`).join("\n") + "\n";
        const r = capText(exact);
        expect(r.hiddenLines).toBe(0);
        expect(r.text).toBe(exact);
    });
});

describe("capChunksByLines", () => {
    const C = (content: string) => ({ content });
    const lines = (n: number, p = "x") => Array.from({ length: n }, (_, i) => `${p}${i}`).join("\n");

    it("returns the same array reference when under budget", () => {
        const chunks = [C("a"), C("b\nc")];
        const r = capChunksByLines(chunks, 10);
        expect(r.hiddenLines).toBe(0);
        expect(r.chunks).toBe(chunks);
    });

    it("drops whole chunks past the budget, keeping the tail", () => {
        const chunks = [C("a"), C("b"), C("c"), C("d")]; // 1 line each
        const r = capChunksByLines(chunks, 2);
        expect(r.chunks.map((c) => c.content)).toEqual(["c", "d"]);
        expect(r.hiddenLines).toBe(2); // a + b
    });

    it("trims the boundary chunk and counts its hidden lines", () => {
        const chunks = [C("a"), C("b\nc"), C("d\ne")]; // 1, 2, 2 lines
        const r = capChunksByLines(chunks, 3);
        // keep "d\ne" (2); "b\nc" overflows the remaining 1 → trimmed to its last line
        expect(r.chunks.map((c) => c.content)).toEqual(["c", "d\ne"]);
        expect(r.hiddenLines).toBe(2); // "a" (1) + the "b" trimmed from "b\nc" (1)
    });

    it("trims a single oversized chunk and reports every hidden line", () => {
        const chunks = [C("a"), C(lines(5000))];
        const r = capChunksByLines(chunks, 1000);
        expect(r.chunks).toHaveLength(1);
        const kept = r.chunks[0].content.split("\n");
        expect(kept).toHaveLength(1000);
        expect(kept[kept.length - 1]).toBe("x4999"); // kept the tail
        expect(r.hiddenLines).toBe(4001); // "a" (1) + 4000 trimmed
    });

    it("trims the only chunk when it alone exceeds the budget", () => {
        const r = capChunksByLines([C(lines(3000))], 1000);
        expect(r.chunks).toHaveLength(1);
        expect(r.chunks[0].content.split("\n")).toHaveLength(1000);
        expect(r.hiddenLines).toBe(2000);
    });

    it("retains object identity for whole kept chunks", () => {
        const chunks = [C("a"), C("b"), C("c")];
        const r = capChunksByLines(chunks, 1);
        expect(r.chunks[r.chunks.length - 1]).toBe(chunks[chunks.length - 1]);
        expect(r.hiddenLines).toBe(2); // a + b
    });

    it("keeps tail chunks by reference even when the boundary is trimmed", () => {
        const tail = C("keep");
        const r = capChunksByLines([C(lines(2000)), tail], 1000);
        expect(r.chunks[r.chunks.length - 1]).toBe(tail);
        expect(r.hiddenLines).toBe(1001);
    });

    it("handles an empty list", () => {
        expect(capChunksByLines([], 10)).toEqual({ chunks: [], hiddenLines: 0 });
    });
});
