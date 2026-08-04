// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    capChars,
    capChunksByLines,
    capText,
    collapseSpinnerChunks,
    createChunkCapper,
    createSpinnerCollapser,
    MAX_TOOL_OUTPUT_CHARS,
    MAX_TOOL_OUTPUT_LINES,
} from "./output-cap";

function chunk(content: string, kind: string = "stdout") {
    return { kind, content };
}

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

    it("char-caps a single very long line that fits the line budget", () => {
        const huge = "x".repeat(MAX_TOOL_OUTPUT_CHARS * 2); // one line, ~2 MB
        const r = capText(huge, 1000, "tail");
        expect(r.text.length).toBeLessThan(MAX_TOOL_OUTPUT_CHARS + 50);
        expect(r.text).toContain("truncated");
    });

    it("does not char-cap normal multi-line output under the byte budget", () => {
        const normal = Array.from({ length: 500 }, () => "a".repeat(80)).join("\n"); // ~40 KB
        const r = capText(normal, 1000);
        expect(r.text).toBe(normal);
        expect(r.hiddenLines).toBe(0);
    });
});

describe("capChars", () => {
    it("returns short strings unchanged", () => {
        expect(capChars("hello", 1000)).toBe("hello");
    });

    it("trims an oversized string to its tail with a marker", () => {
        const huge = "x".repeat(2000);
        const r = capChars(huge, 1000);
        expect(r.length).toBeLessThan(1020);
        expect(r).toContain("truncated");
        expect(r.endsWith("x".repeat(1000))).toBe(true);
    });
});

describe("capChunksByLines", () => {
    const C = (content: string) => ({ content });
    const lines = (n: number, p = "x") => Array.from({ length: n }, (_, i) => `${p}${i}`).join("\n");

    it("returns the same array reference when under budget", () => {
        const chunks = [C("a"), C("b\nc")];
        const r = capChunksByLines(chunks, 10);
        expect(r.keptLines).toBe(3); // 1 + 2, all kept
        expect(r.chunks).toBe(chunks);
    });

    it("drops whole chunks past the budget, keeping the tail", () => {
        const chunks = [C("a"), C("b"), C("c"), C("d")]; // 1 line each
        const r = capChunksByLines(chunks, 2);
        expect(r.chunks.map((c) => c.content)).toEqual(["c", "d"]);
        expect(r.keptLines).toBe(2);
    });

    it("trims the boundary chunk to fit the remaining budget", () => {
        const chunks = [C("a"), C("b\nc"), C("d\ne")]; // 1, 2, 2 lines
        const r = capChunksByLines(chunks, 3);
        // keep "d\ne" (2); "b\nc" overflows the remaining 1 → trimmed to its last line
        expect(r.chunks.map((c) => c.content)).toEqual(["c", "d\ne"]);
        expect(r.keptLines).toBe(3);
    });

    it("trims a single oversized chunk instead of rendering it whole", () => {
        const chunks = [C("a"), C(lines(5000))];
        const r = capChunksByLines(chunks, 1000);
        expect(r.chunks).toHaveLength(1);
        const kept = r.chunks[0].content.split("\n");
        expect(kept).toHaveLength(1000);
        expect(kept[kept.length - 1]).toBe("x4999"); // kept the tail
        expect(r.keptLines).toBe(1000);
    });

    it("trims the only chunk when it alone exceeds the budget", () => {
        const r = capChunksByLines([C(lines(3000))], 1000);
        expect(r.chunks).toHaveLength(1);
        expect(r.chunks[0].content.split("\n")).toHaveLength(1000);
        expect(r.keptLines).toBe(1000);
    });

    it("retains object identity for whole kept chunks", () => {
        const chunks = [C("a"), C("b"), C("c")];
        const r = capChunksByLines(chunks, 1);
        expect(r.chunks[r.chunks.length - 1]).toBe(chunks[chunks.length - 1]);
        expect(r.keptLines).toBe(1);
    });

    it("keeps tail chunks by reference even when the boundary is trimmed", () => {
        const tail = C("keep");
        const r = capChunksByLines([C(lines(2000)), tail], 1000);
        expect(r.chunks[r.chunks.length - 1]).toBe(tail);
        expect(r.keptLines).toBe(1000);
    });

    it("bounds a long run of empty chunks (each still renders a node)", () => {
        const chunks = Array.from({ length: 5000 }, () => C(""));
        const r = capChunksByLines(chunks, 1000);
        expect(r.chunks).toHaveLength(1000); // node count bounded, not unlimited
        expect(r.keptLines).toBe(1000);
    });

    it("handles an empty list", () => {
        expect(capChunksByLines([], 10)).toEqual({ chunks: [], keptLines: 0 });
    });
});

describe("createChunkCapper", () => {
    const C = (content: string) => ({ content });
    const lines = (n: number, p = "x") => Array.from({ length: n }, (_, i) => `${p}${i}`).join("\n");

    it("reports no hidden lines while under budget", () => {
        const cap = createChunkCapper(1000);
        expect(cap([C(lines(500))]).hiddenLines).toBe(0);
    });

    it("derives hidden lines as total minus kept", () => {
        const cap = createChunkCapper(1000);
        const r = cap([C(lines(500)), C(lines(800))]); // total 1300, kept 1000
        expect(r.hiddenLines).toBe(300);
    });

    it("counts each appended chunk exactly once across calls (no prefix rescan)", () => {
        const cap = createChunkCapper(1000);
        const chunks = [C(lines(600))];
        expect(cap(chunks).hiddenLines).toBe(0); // total 600
        chunks.push(C(lines(600))); // in-place append → total 1200
        expect(cap(chunks).hiddenLines).toBe(200); // 1200 - 1000
    });

    it("recounts when the stream resets to a shorter array", () => {
        const cap = createChunkCapper(1000);
        cap([C(lines(900)), C(lines(900))]); // total 1800
        expect(cap([C(lines(300))]).hiddenLines).toBe(0); // reset → total 300
    });

    it("resets when handed a different stream of the same length", () => {
        const cap = createChunkCapper(1000);
        expect(cap([C(lines(900)), C(lines(900))]).hiddenLines).toBe(800); // total 1800 - 1000
        expect(cap([C(lines(100)), C(lines(100))]).hiddenLines).toBe(0); // different stream, total 200
    });

    it("counts empty chunks so a long empty run is bounded", () => {
        const cap = createChunkCapper(1000);
        const r = cap(Array.from({ length: 1500 }, () => C("")));
        expect(r.chunks).toHaveLength(1000);
        expect(r.hiddenLines).toBe(500); // 1500 - 1000
    });
});

describe("collapseSpinnerChunks", () => {
    it("collapses a bare-glyph spinner run into a live slot (existing narrow case, regression guard)", () => {
        const r = collapseSpinnerChunks([chunk("⠋"), chunk("⠙"), chunk("⠹")]);
        expect(r.display).toEqual([]);
        expect(r.spinnerSlot).toEqual({ content: "⠹", kind: "stdout" });
    });

    it("freezes a completed bare-glyph run followed by unrelated output", () => {
        const r = collapseSpinnerChunks([chunk("⠋"), chunk("⠙"), chunk("Done!")]);
        expect(r.display).toEqual([{ kind: "stdout", content: "⠙" }, chunk("Done!")]);
        expect(r.spinnerSlot).toBeNull();
    });

    it("collapses a spinner glyph trailing static text on the same line (spec §B — the main gap this closes)", () => {
        const r = collapseSpinnerChunks([
            chunk("Installing deps... ⠋"),
            chunk("Installing deps... ⠙"),
            chunk("Installing deps... ⠹"),
        ]);
        expect(r.display).toEqual([]);
        expect(r.spinnerSlot).toEqual({ content: "Installing deps... ⠹", kind: "stdout" });
    });

    it("collapses a spinner glyph leading static text on the same line", () => {
        const r = collapseSpinnerChunks([chunk("⠋ Loading..."), chunk("⠙ Loading...")]);
        expect(r.spinnerSlot).toEqual({ content: "⠙ Loading...", kind: "stdout" });
    });

    it("collapses changing percentage/progress text with no spinner glyph at all", () => {
        const r = collapseSpinnerChunks([
            chunk("Downloading (12%)"),
            chunk("Downloading (45%)"),
            chunk("Downloading (100%)"),
        ]);
        expect(r.display).toEqual([]);
        expect(r.spinnerSlot).toEqual({ content: "Downloading (100%)", kind: "stdout" });
    });

    it("freezes a completed progress-text run followed by unrelated output", () => {
        const r = collapseSpinnerChunks([
            chunk("Downloading (12%)"),
            chunk("Downloading (100%)"),
            chunk("Extracting archive"),
        ]);
        expect(r.display).toEqual([
            { kind: "stdout", content: "Downloading (100%)" },
            chunk("Extracting archive"),
        ]);
        expect(r.spinnerSlot).toBeNull();
    });

    it("does NOT collapse two unrelated consecutive lines with only coincidental partial overlap", () => {
        const r = collapseSpinnerChunks([chunk("Compiling src/main.rs"), chunk("Compiling src/lib.rs")]);
        expect(r.display).toEqual([chunk("Compiling src/main.rs"), chunk("Compiling src/lib.rs")]);
        expect(r.spinnerSlot).toBeNull();
    });

    it("does NOT collapse two genuinely different short lines", () => {
        const r = collapseSpinnerChunks([chunk("npm WARN deprecated foo@1.0.0"), chunk("npm WARN deprecated bar@2.0.0")]);
        expect(r.spinnerSlot).toBeNull();
        expect(r.display.length).toBeGreaterThanOrEqual(1);
    });

    it("no spinner/progress content in the output is a pure passthrough (no overhead behavior change)", () => {
        const r = collapseSpinnerChunks([chunk("line one"), chunk("line two"), chunk("line three")]);
        expect(r.display).toEqual([chunk("line one"), chunk("line two"), chunk("line three")]);
        expect(r.spinnerSlot).toBeNull();
    });

    it("multiple disjoint redraw runs each collapse independently", () => {
        const r = collapseSpinnerChunks([
            chunk("Step 1... ⠋"),
            chunk("Step 1... ⠙"),
            chunk("Step 1 done"),
            chunk("Step 2... ⠋"),
            chunk("Step 2... ⠙"),
        ]);
        expect(r.display).toEqual([
            { kind: "stdout", content: "Step 1... ⠙" },
            chunk("Step 1 done"),
        ]);
        expect(r.spinnerSlot).toEqual({ content: "Step 2... ⠙", kind: "stdout" });
    });

    it("does NOT collapse ordinary sequential lines that happen to differ only by a bare number (reagent P1, PR #2330 — bare digit runs must not be stripped like percentages)", () => {
        const r = collapseSpinnerChunks([chunk("case 1 passed"), chunk("case 2 passed"), chunk("case 3 passed")]);
        expect(r.display).toEqual([chunk("case 1 passed"), chunk("case 2 passed"), chunk("case 3 passed")]);
        expect(r.spinnerSlot).toBeNull();
    });

    it("does NOT hang or throw on a very long non-progress line (reagent P1, PR #2330 — unbounded Levenshtein on an ~8KiB bashwrap chunk)", () => {
        const huge = "x".repeat(9000);
        const start = performance.now();
        const r = collapseSpinnerChunks([chunk(huge), chunk(huge + "y")]);
        expect(performance.now() - start).toBeLessThan(200);
        expect(r.display).toEqual([chunk(huge), chunk(huge + "y")]);
        expect(r.spinnerSlot).toBeNull();
    });
});

describe("createSpinnerCollapser (incremental sibling of collapseSpinnerChunks)", () => {
    // Cross-check helper: for every prefix length of `chunks`, the
    // incremental collapser fed one chunk at a time must match the batch
    // function run fresh on that same prefix. This is the real correctness
    // guarantee — reagent P1 (PR #2330) is about complexity, not behavior,
    // so every existing collapseSpinnerChunks scenario above is replayed
    // here incrementally and must produce identical results at every step.
    function assertMatchesBatchAtEveryStep(chunks: ReturnType<typeof chunk>[]) {
        const collapse = createSpinnerCollapser<ReturnType<typeof chunk>>();
        for (let n = 1; n <= chunks.length; n++) {
            const prefix = chunks.slice(0, n);
            const incremental = collapse(prefix);
            const batch = collapseSpinnerChunks(prefix);
            expect(incremental).toEqual(batch);
        }
    }

    it("matches the batch function at every prefix — bare-glyph run", () => {
        assertMatchesBatchAtEveryStep([chunk("⠋"), chunk("⠙"), chunk("⠹")]);
    });

    it("matches the batch function at every prefix — completed bare-glyph run", () => {
        assertMatchesBatchAtEveryStep([chunk("⠋"), chunk("⠙"), chunk("Done!")]);
    });

    it("matches the batch function at every prefix — glyph trailing static text", () => {
        assertMatchesBatchAtEveryStep([
            chunk("Installing deps... ⠋"),
            chunk("Installing deps... ⠙"),
            chunk("Installing deps... ⠹"),
        ]);
    });

    it("matches the batch function at every prefix — percentage progress, no glyph", () => {
        assertMatchesBatchAtEveryStep([
            chunk("Downloading (12%)"),
            chunk("Downloading (45%)"),
            chunk("Downloading (100%)"),
        ]);
    });

    it("matches the batch function at every prefix — completed progress run + unrelated output", () => {
        assertMatchesBatchAtEveryStep([
            chunk("Downloading (12%)"),
            chunk("Downloading (100%)"),
            chunk("Extracting archive"),
        ]);
    });

    it("matches the batch function at every prefix — no false-positive collapse", () => {
        assertMatchesBatchAtEveryStep([chunk("Compiling src/main.rs"), chunk("Compiling src/lib.rs")]);
    });

    it("matches the batch function at every prefix — multiple disjoint runs", () => {
        assertMatchesBatchAtEveryStep([
            chunk("Step 1... ⠋"),
            chunk("Step 1... ⠙"),
            chunk("Step 1 done"),
            chunk("Step 2... ⠋"),
            chunk("Step 2... ⠙"),
        ]);
    });

    it("retroactively absorbs a standalone-looking chunk once a later chunk turns it into a run (the case incrementality risks getting wrong)", () => {
        // At the point "text" arrives, it doesn't yet know whether it starts
        // a run (no next chunk exists) — collapseSpinnerChunks would show it
        // in `display` for that snapshot. Once "text redraw" arrives and
        // matches it, the ORIGINAL algorithm (rerun from scratch) would
        // retroactively group them instead of showing "text" standalone.
        // The incremental version must reproduce that, not freeze its
        // earlier "shown standalone" guess.
        assertMatchesBatchAtEveryStep([
            chunk("⠋"), chunk("⠙"),        // a bare-glyph run
            chunk("text"),                  // ambiguous while it's the last chunk
            chunk("text redraw"),           // retroactively joins "text" into a run
            chunk("unrelated"),             // breaks the run, freezes it
        ]);
    });

    it("produces the exact same result as the batch function on the full array in one shot", () => {
        const chunks = [
            chunk("⠋"), chunk("⠙"), chunk("Step 1 done"),
            chunk("Downloading (1%)"), chunk("Downloading (99%)"),
        ];
        const collapse = createSpinnerCollapser<ReturnType<typeof chunk>>();
        expect(collapse(chunks)).toEqual(collapseSpinnerChunks(chunks));
    });

    it("recounts when the stream resets to a shorter array (mirrors createChunkCapper)", () => {
        const collapse = createSpinnerCollapser<ReturnType<typeof chunk>>();
        collapse([chunk("⠋"), chunk("⠙"), chunk("Done!")]);
        const shorter = [chunk("Compiling")];
        expect(collapse(shorter)).toEqual(collapseSpinnerChunks(shorter));
    });

    it("resets when handed a different stream of the same length", () => {
        const collapse = createSpinnerCollapser<ReturnType<typeof chunk>>();
        collapse([chunk("⠋"), chunk("⠙")]);
        const different = [chunk("a"), chunk("b")];
        expect(collapse(different)).toEqual(collapseSpinnerChunks(different));
    });

    it("only re-examines newly appended chunks, not the whole prior window (in-place append)", () => {
        const collapse = createSpinnerCollapser<ReturnType<typeof chunk>>();
        const chunks = [chunk("Compiling a"), chunk("Compiling b")];
        const first = collapse(chunks);
        expect(first).toEqual(collapseSpinnerChunks(chunks));
        chunks.push(chunk("⠋"), chunk("⠙"));
        const second = collapse(chunks);
        expect(second).toEqual(collapseSpinnerChunks(chunks));
    });
});
