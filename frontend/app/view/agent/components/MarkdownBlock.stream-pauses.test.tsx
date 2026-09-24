// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pause in the middle of a stream must not re-parse the whole message.
 *
 * MarkdownBlock cannot know when a stream has ended; it infers "settled" from
 * STREAM_RENDER_MS (90 ms) without an update and then commits the final,
 * syntax-highlighted render. Real streams pause longer than that all the time
 * (network chunking, a slow tool call before more text), so that trailing
 * render fires mid-stream — and the next chunk goes back to un-highlighted.
 *
 * Before this was fixed, each of those flips changed the processor the frozen
 * prefix was rendered with, which invalidated it: the WHOLE message was
 * re-parsed once highlighted at the pause and once plain on the next chunk.
 * With a pause every few chunks that is the O(n²) re-parse incremental
 * rendering exists to remove, back through a side door — profiled as the
 * largest single cost of a stream flush
 * (docs/specs/TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md §3.4).
 *
 * Counts, not milliseconds — see markdown-render-work.test.tsx for why.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MarkdownBlock } from "./MarkdownBlock";
import { Markdown, __markdownRenderStats, __resetMarkdownRenderStats } from "@/app/element/markdown";
import type { MarkdownNode } from "../types";

beforeEach(() => {
    vi.useFakeTimers();
});

afterEach(() => {
    vi.useRealTimers();
    cleanup();
});

/** Headings, prose and a fenced code block per section — the code block is
 *  what highlighting changes, so a stale un-highlighted frozen prefix shows. */
const SAMPLE = Array.from(
    { length: 30 },
    (_, i) =>
        `## Section ${i}\n\nSome prose for section ${i} with \`inline\` code.\n\n` +
        "```ts\nconst x" + i + ": number = " + i + ";\n```\n\n",
).join("");

/**
 * Streams SAMPLE into a MarkdownBlock in `chunks` updates, 30 ms apart, with a
 * `pauseMs` gap after every `pauseEvery`-th chunk. Ends settled.
 */
function stream(chunks: number, pauseEvery: number, pauseMs: number) {
    const [node, setNode] = createSignal<MarkdownNode>({ type: "markdown", id: "md-1", content: "" });
    const result = render(() => <MarkdownBlock node={node()} />);
    const step = Math.ceil(SAMPLE.length / chunks);
    for (let i = 1; i <= chunks; i++) {
        setNode({ type: "markdown", id: "md-1", content: SAMPLE.slice(0, Math.min(i * step, SAMPLE.length)) });
        vi.advanceTimersByTime(i % pauseEvery === 0 ? pauseMs : 30);
    }
    vi.advanceTimersByTime(500); // settle: the final highlighted render
    return result;
}

describe("MarkdownBlock — pauses mid-stream", () => {
    it("control: an unpaused stream parses each byte about once", () => {
        __resetMarkdownRenderStats();
        stream(40, Number.POSITIVE_INFINITY, 0);
        const ratio = __markdownRenderStats.parsedChars / SAMPLE.length;
        expect(ratio).toBeLessThan(2.5);
    });

    it("pauses longer than the settle window do not re-parse the frozen prefix", () => {
        __resetMarkdownRenderStats();
        stream(40, 4, 200); // a 200 ms pause every 4th chunk: ten "settles" mid-stream
        const ratio = __markdownRenderStats.parsedChars / SAMPLE.length;
        expect(
            ratio,
            `parsed ${__markdownRenderStats.parsedChars} chars for a ${SAMPLE.length}-char message ` +
                `(${ratio.toFixed(1)}x) over ${__markdownRenderStats.commits} commits. Each mid-stream ` +
                `settle must re-render only the open tail, not the whole message.`,
        ).toBeLessThan(2.5);
    });

    /**
     * The settle no longer re-renders the whole message, so the final DOM is
     * the incrementally assembled one. It must match a from-scratch render in
     * everything that renders. The one allowed difference is the same one
     * markdown-incremental.test.ts's split-equivalence cases allow: a
     * whitespace-only text node BETWEEN top-level blocks, which a segment seam
     * drops. Those blocks are normal-flow `display: block` with
     * `white-space: normal` (`.agent-markdown-block .markdown > .content`), so
     * such a node generates no box. Whitespace inside any element — code
     * blocks above all — is still compared exactly.
     */
    it("the settled result matches rendering the finished message from scratch", () => {
        const { container } = stream(40, 4, 200);
        const reference = render(() => <Markdown text={SAMPLE} scrollable={false} />);
        const blocksHtml = (root: Element): string => {
            const content = root.querySelector(".markdown > .content")!.cloneNode(true) as Element;
            for (const n of [...content.childNodes]) {
                if (n.nodeType === Node.TEXT_NODE && n.textContent!.trim() === "") n.remove();
            }
            // Heading ids carry a per-instance random prefix; compare the rest.
            return content.innerHTML.replace(/id="[^"]*"/g, 'id=""');
        };
        const got = blocksHtml(container);
        expect(got).toContain("hljs"); // highlighted, not the plain streaming render
        expect(got).toBe(blocksHtml(reference.container));
    });
});
