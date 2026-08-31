// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMessageBlock — peek tooltip added by
 * SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25 (this node kind had
 * none before; it never surfaces its own timestamp anywhere in the UI).
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AgentMessageBlock } from "./AgentMessageBlock";
import type { AgentMessageNode } from "../types";

afterEach(() => cleanup());

const node: AgentMessageNode = {
    type: "agent_message",
    id: "am-1",
    from: "claude-1",
    to: "reviewer",
    message: "hello there",
    method: "mux",
    direction: "incoming",
    timestamp: Date.now() - 65_000,
    collapsed: true,
    summary: "📨 claude-1 → reviewer (mux)",
};

const hover = (container: HTMLElement) => {
    const root = container.querySelector(".agent-message-block") as HTMLElement;
    fireEvent.mouseEnter(root);
    vi.advanceTimersByTime(100);
};

describe("AgentMessageBlock — peek tooltip", () => {
    it("shows time + estimate on hover, regardless of collapsed state", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <AgentMessageBlock node={node} collapsed={true} onToggle={() => {}} />
            ));
            hover(container);
            const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
            expect(metaLines.length).toBe(2);
            expect(metaLines[0].textContent).toMatch(/\d{1,2}:\d{2}:\d{2} (?:AM|PM) · 1m ago/);
            expect(metaLines[1].textContent).toMatch(/~\d+ tok \(est\.\)/);
        } finally {
            vi.useRealTimers();
        }
    });

    it("hides on mouseleave", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <AgentMessageBlock node={node} collapsed={true} onToggle={() => {}} />
            ));
            hover(container);
            expect(document.body.querySelector(".agent-node-peek-overlay")).not.toBeNull();
            fireEvent.mouseLeave(container.querySelector(".agent-message-block") as HTMLElement);
            expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });

    it("does not fire onToggle on hover — click and hover stay independent", () => {
        vi.useFakeTimers();
        try {
            const onToggle = vi.fn();
            const { container } = render(() => (
                <AgentMessageBlock node={node} collapsed={true} onToggle={onToggle} />
            ));
            hover(container);
            expect(onToggle).not.toHaveBeenCalled();
        } finally {
            vi.useRealTimers();
        }
    });
});
