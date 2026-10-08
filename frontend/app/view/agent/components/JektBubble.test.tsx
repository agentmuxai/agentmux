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

// Held open when it arrives live, like a tool that finishes on screen; never
// for a loaded one, or for a sensitive one (open by default anyway).
// REPORT_JEKT_COLLAPSE_AND_PREVIEW_SKID_2026_10_07.md §2.1, D1–D2.
describe("JektBubble — the live-arrival hold", () => {
    const at = (timestamp: number, tier: JektMessageNode["tier"] = "coord"): JektMessageNode => ({ ...node, timestamp, tier });

    it("asks to be held open when it arrives live", () => {
        const onHoldOpen = vi.fn();
        render(() => <JektBubble node={at(Date.now())} collapsed={false} onToggle={() => {}} onHoldOpen={onHoldOpen} />);
        expect(onHoldOpen).toHaveBeenCalledTimes(1);
    });

    it("does not for a jekt loaded from history", () => {
        const onHoldOpen = vi.fn();
        render(() => <JektBubble node={at(Date.now() - 60_000)} collapsed={true} onToggle={() => {}} onHoldOpen={onHoldOpen} />);
        expect(onHoldOpen).not.toHaveBeenCalled();
    });

    it("does not for a sensitive jekt", () => {
        const onHoldOpen = vi.fn();
        render(() => (
            <JektBubble node={at(Date.now(), "sensitive")} collapsed={false} onToggle={() => {}} onHoldOpen={onHoldOpen} />
        ));
        expect(onHoldOpen).not.toHaveBeenCalled();
    });

    it("gives its boxes the shared preview-box scroll hand-off", () => {
        const { container } = render(() => <JektBubble node={node} collapsed={false} onToggle={() => {}} />);
        expect(container.querySelector(".agent-jekt-body")!.classList.contains("scroll-handoff-box")).toBe(true);
    });
});

describe("JektBubble — peek tooltip", () => {
    it("shows time + estimate on hover", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <JektBubble node={node} collapsed={true} onToggle={() => {}} />
            ));
            hover(container);
            const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
            // One line: time and tokens side by side (PeekMetaRow).
            expect(metaLines.length).toBe(1);
            expect(metaLines[0].querySelector(".agent-node-peek-tooltip-time")?.textContent).toMatch(/\d{1,2}:\d{2}:\d{2} (?:AM|PM) · 1m ago/);
            expect(metaLines[0].querySelector(".agent-node-peek-tooltip-tokens")?.textContent).toMatch(/~\d+ tok \(est\.\)/);
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
