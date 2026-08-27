// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * JektBubble — peek tooltip added by
 * SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25. Adds a relative
 * "time ago" the expanded meta row's plain `toLocaleString()` doesn't have.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { JektBubble } from "./JektBubble";
import type { JektMessageNode } from "../types";

afterEach(() => cleanup());

const node: JektMessageNode = {
    type: "jekt_message",
    id: "jm-1",
    from: "github-consumer",
    to: "naki",
    message: "PR #2676 reviewed — LGTM",
    raw: "[JEKT:...][/JEKT]",
    tier: "coord",
    deliveryTier: "wan",
    trust: "network-claimed",
    msgId: "inj-1",
    priority: "normal",
    direction: "incoming",
    timestamp: Date.now() - 65_000,
};

const hover = (container: HTMLElement) => {
    const root = container.querySelector(".agent-jekt-bubble") as HTMLElement;
    fireEvent.mouseEnter(root);
    vi.advanceTimersByTime(100);
};

describe("JektBubble — peek tooltip", () => {
    it("shows time + estimate on hover", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <JektBubble node={node} collapsed={true} onToggle={() => {}} />
            ));
            hover(container);
            const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
            expect(metaLines.length).toBe(2);
            expect(metaLines[0].textContent).toMatch(/\d{2}:\d{2}:\d{2} · 1m ago/);
            expect(metaLines[1].textContent).toMatch(/~\d+ tok \(est\.\)/);
        } finally {
            vi.useRealTimers();
        }
    });

    it("hides on mouseleave", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <JektBubble node={node} collapsed={true} onToggle={() => {}} />
            ));
            hover(container);
            expect(document.body.querySelector(".agent-node-peek-overlay")).not.toBeNull();
            fireEvent.mouseLeave(container.querySelector(".agent-jekt-bubble") as HTMLElement);
            expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });
});
