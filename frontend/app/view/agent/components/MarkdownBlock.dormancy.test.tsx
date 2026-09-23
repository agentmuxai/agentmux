// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A backgrounded agent pane must not spend the main thread rendering content
 * nobody can see.
 *
 * WHY THIS MATTERS
 *
 * Keep-alive switches pane-stack members by visibility, not existence
 * (`pane-leaf-chrome.tsx`), so a backgrounded agent tab stays MOUNTED. With no
 * gate on the render path, four streaming agents means four panes parsing
 * markdown on one main thread — and the one you are typing into gets a
 * quarter of it. See ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md §4.
 *
 * These assert on the deterministic work counters rather than wall-clock, for
 * the same reason as markdown-render-work.test.tsx: a timing gate on shared CI
 * runners flakes until someone deletes it.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MarkdownBlock } from "./MarkdownBlock";
import { AgentDormancyProvider } from "../agent-dormancy";
import { __markdownRenderStats, __resetMarkdownRenderStats } from "@/app/element/markdown";
import type { MarkdownNode } from "../types";

// MarkdownBlock throttles commits to STREAM_RENDER_MS (90ms) and defers the
// highlighted render to a trailing timeout. Without advancing timers, a
// synchronous loop of updates commits NOTHING — which would make a dormancy
// assertion pass vacuously, for the wrong reason. Fake timers (which vitest
// also applies to `performance.now`, the throttle's own clock) make each
// update a real commit.
beforeEach(() => {
    vi.useFakeTimers();
});

afterEach(() => {
    vi.useRealTimers();
    cleanup();
});

const LONG = "A paragraph of streamed assistant output that costs real parse time.\n\n".repeat(12);

/** Streams `chunks` growing updates into a MarkdownBlock at a given dormancy. */
function streamAtDormancy(dormant: boolean, chunks: number) {
    const [node, setNode] = createSignal<MarkdownNode>({ type: "markdown", id: "md-1", content: "" });
    const [isDormant] = createSignal(dormant);

    const result = render(() => (
        <AgentDormancyProvider dormant={isDormant}>
            <MarkdownBlock node={node()} />
        </AgentDormancyProvider>
    ));

    const step = Math.ceil(LONG.length / chunks);
    for (let i = 1; i <= chunks; i++) {
        setNode({ type: "markdown", id: "md-1", content: LONG.slice(0, Math.min(i * step, LONG.length)) });
        vi.advanceTimersByTime(100); // past STREAM_RENDER_MS, so each update commits
    }
    return result;
}

describe("MarkdownBlock — dormant panes don't render", () => {
    it("does no markdown parse work while the pane is backgrounded", () => {
        __resetMarkdownRenderStats();
        streamAtDormancy(true, 20);

        expect(
            __markdownRenderStats.parsedChars,
            `a backgrounded pane parsed ${__markdownRenderStats.parsedChars} chars across ` +
                `${__markdownRenderStats.commits} commits — it should defer all of it until shown`,
        ).toBe(0);
    });

    it("a visible pane still renders normally (control)", () => {
        __resetMarkdownRenderStats();
        streamAtDormancy(false, 20);

        // The control matters: without it, a gate that accidentally disabled
        // ALL rendering would pass the test above and look like a fix.
        expect(__markdownRenderStats.parsedChars).toBeGreaterThan(0);
    });

    it("renders the latest content once the pane becomes visible", () => {
        const [node, setNode] = createSignal<MarkdownNode>({ type: "markdown", id: "md-1", content: "" });
        const [dormant, setDormant] = createSignal(true);

        const { container } = render(() => (
            <AgentDormancyProvider dormant={dormant}>
                <MarkdownBlock node={node()} />
            </AgentDormancyProvider>
        ));

        setNode({ type: "markdown", id: "md-1", content: "# Arrived while hidden\n\nBody text." });
        vi.advanceTimersByTime(100);
        expect(container.textContent).not.toContain("Arrived while hidden");

        // Deferred, not dropped: showing the pane must surface everything that
        // accumulated while it was hidden, without needing another update.
        setDormant(false);
        vi.advanceTimersByTime(100);
        expect(container.textContent).toContain("Arrived while hidden");
    });
});
