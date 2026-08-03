// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MarkdownBlock — thinking-clump peek tooltip
 * (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md §2.4). Regular (non-
 * thinking) markdown and the canceled-thinking path are unchanged by this
 * feature; covered here only enough to confirm neither regressed.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";

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

describe("MarkdownBlock — regular text (unaffected)", () => {
    it("renders plain content with no tooltip anchor", () => {
        const node: MarkdownNode = { type: "markdown", id: "md-2", content: "Hello" };
        const { container, unmount } = render(() => <MarkdownBlock node={node} />);
        expect(container.querySelector(".thinking-block")).toBeNull();
        unmount();
    });
});

describe("MarkdownBlock — thinking-clump peek tooltip", () => {
    it("shows exact time + time-ago + an estimated token count when the node has a timestamp", () => {
        const timed: MarkdownNode = { ...thinkingNode, timestamp: Date.now() - 65_000 };
        const { container, unmount } = render(() => <MarkdownBlock node={timed} />);
        const anchor = container.querySelector(".thinking-block") as HTMLElement;
        expect(anchor).not.toBeNull();
        fireEvent.mouseEnter(anchor);
        const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
        expect(metaLines.length).toBe(2);
        expect(metaLines[0].textContent).toMatch(/\d{2}:\d{2}:\d{2} · 1m ago/);
        expect(metaLines[1].textContent).toMatch(/~\d+ tok \(est\.\)/);
        unmount();
    });

    it("shows only the estimate line when the node has no timestamp", () => {
        const untimed: MarkdownNode = { ...thinkingNode, timestamp: undefined };
        const { container, unmount } = render(() => <MarkdownBlock node={untimed} />);
        const anchor = container.querySelector(".thinking-block") as HTMLElement;
        fireEvent.mouseEnter(anchor);
        const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
        expect(metaLines.length).toBe(1);
        expect(metaLines[0].textContent).toMatch(/~\d+ tok \(est\.\)/);
        unmount();
    });

    it("a canceled thinking clump renders its own collapsed header, no peek tooltip anchor", () => {
        const canceled: MarkdownNode = {
            ...thinkingNode,
            metadata: { thinking: true, canceled: true },
        };
        const { getByText, unmount } = render(() => <MarkdownBlock node={canceled} />);
        expect(getByText("Canceled — partial thought")).toBeInTheDocument();
        unmount();
    });
});
