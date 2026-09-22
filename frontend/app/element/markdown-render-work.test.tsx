// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Streaming-render work invariants for <Markdown>.
 *
 * WHY THESE ASSERT ON COUNTS, NOT MILLISECONDS
 *
 * The regression being guarded is algorithmic, and the repo has already taken
 * the position (Discussion #1161) that absolute wall-clock gates on shared CI
 * runners are theater — they flake until someone disables them, which is worse
 * than no gate at all. So these assert on deterministic work counters
 * (`__markdownRenderStats`): machine-independent, non-flaky, and they fail for
 * a reason a reader can act on.
 *
 * WHAT WENT WRONG, SO NOBODY RE-INTRODUCES IT
 *
 * `<Markdown>` re-parses the ENTIRE text on every commit. During agent
 * streaming the text grows ~11 commits/sec, so rendering one 32KB response
 * burned 1.7s of main thread in blocks up to 92ms and starved keystrokes in the
 * composer — measured in
 * `docs/analysis/ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md` §6, with
 * `tools/tests/bench-markdown-parse.mjs`. Separately, the unified processor was
 * being rebuilt inside the render memo, adding a fixed ~4ms to every one of
 * those commits.
 *
 * This is the third appearance of this bug class in this codebase (see the
 * "not silky" history in SPEC_TERM_DOUBLE_RAF_TEAROUT_2026_05_30). Twice it was
 * fixed, documented, and reintroduced anyway. Hence executable guards.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";

import { Markdown, __markdownRenderStats, __resetMarkdownRenderStats } from "./markdown";

afterEach(() => {
    cleanup();
});

/** Streams `text` in `chunks` slices, committing each one, like an agent response. */
function streamInto(text: string, chunks: number) {
    const [content, setContent] = createSignal("");
    const result = render(() => <Markdown text={content()} scrollable={false} />);
    const step = Math.ceil(text.length / chunks);
    for (let i = 1; i <= chunks; i++) {
        setContent(text.slice(0, Math.min(i * step, text.length)));
    }
    return result;
}

const SAMPLE = Array.from(
    { length: 40 },
    (_, i) => `## Section ${i}\n\nSome prose for section ${i} with \`inline\` code.\n\n`,
).join("");

describe("Markdown streaming-render work invariants", () => {
    it("builds the unified processor once, not once per commit", () => {
        __resetMarkdownRenderStats();
        streamInto(SAMPLE, 25);

        // The processor depends only on `highlight` and the slug prefix —
        // never on the text — so streaming must not rebuild it. It used to be
        // constructed inside the render memo, making this equal to the commit
        // count and costing ~4ms every commit.
        expect(__markdownRenderStats.commits).toBeGreaterThan(1);
        expect(__markdownRenderStats.processorBuilds).toBe(1);
    });

    it("does not rebuild the processor when only the text changes", () => {
        __resetMarkdownRenderStats();
        const [content, setContent] = createSignal("first");
        render(() => <Markdown text={content()} scrollable={false} />);
        const afterMount = __markdownRenderStats.processorBuilds;

        setContent("second, quite different text");
        setContent("third, different again");

        expect(__markdownRenderStats.processorBuilds).toBe(afterMount);
    });

    /**
     * THE load-bearing invariant: a streaming commit may only re-parse the
     * trailing open block, never the whole message.
     *
     * Before incremental parsing this was ~13x the message length at 25
     * commits, and grew with commit count — the O(n²) that burned 1.7s of main
     * thread on a 32KB response. If this assertion starts failing, something
     * has reverted to a full re-parse per commit; fix that rather than raising
     * the bound.
     */
    it("only re-parses the trailing block, not the whole message", () => {
        __resetMarkdownRenderStats();
        streamInto(SAMPLE, 25);

        const ratio = __markdownRenderStats.parsedChars / SAMPLE.length;
        expect(
            ratio,
            `parsed ${__markdownRenderStats.parsedChars} chars to render a ${SAMPLE.length}-char ` +
                `message over ${__markdownRenderStats.commits} commits (${ratio.toFixed(1)}x). ` +
                `Incremental parsing should keep this near 1x; a full re-parse per commit is ~13x.`,
        ).toBeLessThan(2);
    });

    /**
     * Scaling, not a fixed budget: doubling the commit count must not
     * meaningfully increase total work, because each byte is still parsed once.
     * This is the machine-independent way to assert "not O(n²)" — it compares
     * two counts from the same run rather than against any wall-clock number.
     */
    it("total parse work does not grow with commit count", () => {
        __resetMarkdownRenderStats();
        streamInto(SAMPLE, 10);
        const coarse = __markdownRenderStats.parsedChars;

        __resetMarkdownRenderStats();
        streamInto(SAMPLE, 40);
        const fine = __markdownRenderStats.parsedChars;

        expect(
            fine / coarse,
            `streaming the same message in 4x as many commits parsed ${fine} chars vs ${coarse} ` +
                `(${(fine / coarse).toFixed(1)}x). Under a full re-parse this scales with commit ` +
                `count; under incremental parsing it should stay flat.`,
        ).toBeLessThan(2);
    });
});
