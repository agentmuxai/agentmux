// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MarkdownBlock — peek tooltip (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md
 * §2.4, extended to regular assistant text by
 * SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25 — "regular assistant
 * text... remain out of scope" no longer holds). Thinking clumps and regular
 * text now share the same anchor/overlay; the canceled-thinking path is
 * unchanged, covered here only enough to confirm it didn't regress.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { MarkdownBlock } from "./MarkdownBlock";
import type { MarkdownNode } from "../types";

afterEach(() => {
    cleanup();
});

const thinkingNode: MarkdownNode = {
    type: "markdown",
    id: "md-1",
    content: "Let me think about this...",
    metadata: { thinking: true },
};

// The peek overlay is Portal-rendered at document.body (PeekOverlay.tsx
// — escapes each virtualized row's own CSS stacking context, see that
// file's doc comment), so it lives in `document.body`, not `container`.
// A real 50ms enter-delay gates its DOM presence, so these tests use fake
// timers and advance past it.
const hoverBlock = (container: HTMLElement) => {
    const anchor = container.querySelector(".agent-markdown-peek-anchor") as HTMLElement;
    fireEvent.mouseEnter(anchor);
    vi.advanceTimersByTime(100);
};

describe("MarkdownBlock — regular (non-thinking) text now gets a peek too", () => {
    it("renders plain content with no thinking-block styling", () => {
        const node: MarkdownNode = { type: "markdown", id: "md-2", content: "Hello" };
        const { container, unmount } = render(() => <MarkdownBlock node={node} />);
        expect(container.querySelector(".thinking-block")).toBeNull();
        unmount();
    });

    it("shows a peek with time + estimate on hover, same as a thinking clump", () => {
        const node: MarkdownNode = { type: "markdown", id: "md-2", content: "Hello", timestamp: Date.now() - 65_000 };
        vi.useFakeTimers();
        try {
            const { container } = render(() => <MarkdownBlock node={node} />);
            expect(container.querySelector(".agent-markdown-peek-anchor")).not.toBeNull();
            hoverBlock(container);
            const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
            expect(metaLines.length).toBe(2);
            expect(metaLines[0].textContent).toMatch(/\d{2}:\d{2}:\d{2} · 1m ago/);
            expect(metaLines[1].textContent).toMatch(/~\d+ tok \(est\.\)/);
        } finally {
            vi.useRealTimers();
        }
    });
});

describe("MarkdownBlock — thinking-clump peek tooltip", () => {
    it("shows exact time + time-ago + an estimated token count when the node has a timestamp", () => {
        const timed: MarkdownNode = { ...thinkingNode, timestamp: Date.now() - 65_000 };
        vi.useFakeTimers();
        try {
            const { container } = render(() => <MarkdownBlock node={timed} />);
            expect(container.querySelector(".thinking-block")).not.toBeNull();
            hoverBlock(container);
            const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
            expect(metaLines.length).toBe(2);
            expect(metaLines[0].textContent).toMatch(/\d{2}:\d{2}:\d{2} · 1m ago/);
            expect(metaLines[1].textContent).toMatch(/~\d+ tok \(est\.\)/);
        } finally {
            vi.useRealTimers();
        }
    });

    it("shows only the estimate line when the node has no timestamp", () => {
        const untimed: MarkdownNode = { ...thinkingNode, timestamp: undefined };
        vi.useFakeTimers();
        try {
            const { container } = render(() => <MarkdownBlock node={untimed} />);
            hoverBlock(container);
            const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
            expect(metaLines.length).toBe(1);
            expect(metaLines[0].textContent).toMatch(/~\d+ tok \(est\.\)/);
        } finally {
            vi.useRealTimers();
        }
    });

    it("does not subscribe to the shared ticker before being hovered", () => {
        const timed: MarkdownNode = { ...thinkingNode, timestamp: Date.now() - 65_000 };
        const { container } = render(() => <MarkdownBlock node={timed} />);
        // No hover fired — peekTimeText's memo must short-circuit before
        // reading peekTick(), so the overlay shows nothing yet.
        const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
        expect(metaLines.length).toBe(0);
        expect(container.querySelector(".thinking-block")).not.toBeNull();
    });

    it("a canceled thinking clump renders its own collapsed header, no peek tooltip anchor", () => {
        const canceled: MarkdownNode = {
            ...thinkingNode,
            metadata: { thinking: true, canceled: true },
        };
        const { getByText, container, unmount } = render(() => <MarkdownBlock node={canceled} />);
        expect(getByText("Canceled — partial thought")).toBeInTheDocument();
        expect(container.querySelector(".agent-markdown-peek-anchor")).toBeNull();
        unmount();
    });
});
