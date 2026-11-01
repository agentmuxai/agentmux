// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * OriginContainer jekt variant — peek tooltip added by
 * SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25. Adds a relative
 * "time ago" the expanded meta row's plain `toLocaleString()` doesn't have.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { OriginContainer } from "./OriginContainer";
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
    const root = container.querySelector(".agent-origin--agent") as HTMLElement;
    fireEvent.mouseEnter(root);
    vi.advanceTimersByTime(100);
};

// Held open when it arrives live, like a tool that finishes on screen; never
// for a loaded one, or for a sensitive one (open by default anyway).
// REPORT_JEKT_COLLAPSE_AND_PREVIEW_SKID_2026_10_07.md §2.1, D1–D2.
describe("OriginContainer jekt variant — the live-arrival hold", () => {
    const at = (timestamp: number, tier: JektMessageNode["tier"] = "coord"): JektMessageNode => ({ ...node, timestamp, tier });

    it("asks to be held open when it arrives live", () => {
        const onHoldOpen = vi.fn();
        render(() => <OriginContainer node={at(Date.now())} collapsed={false} onToggle={() => {}} onHoldOpen={onHoldOpen} />);
        expect(onHoldOpen).toHaveBeenCalledTimes(1);
    });

    it("counts a held jekt from when it was delivered, not when it was sent", () => {
        const onHoldOpen = vi.fn();
        const held: JektMessageNode = { ...node, timestamp: Date.now() - 600_000, heldForSecs: 600 };
        render(() => <OriginContainer node={held} collapsed={false} onToggle={() => {}} onHoldOpen={onHoldOpen} />);
        expect(onHoldOpen).toHaveBeenCalledTimes(1);
    });

    it("does not for a jekt loaded from history", () => {
        const onHoldOpen = vi.fn();
        render(() => <OriginContainer node={at(Date.now() - 60_000)} collapsed={true} onToggle={() => {}} onHoldOpen={onHoldOpen} />);
        expect(onHoldOpen).not.toHaveBeenCalled();
    });

    it("does not for a sensitive jekt", () => {
        const onHoldOpen = vi.fn();
        render(() => (
            <OriginContainer node={at(Date.now(), "sensitive")} collapsed={false} onToggle={() => {}} onHoldOpen={onHoldOpen} />
        ));
        expect(onHoldOpen).not.toHaveBeenCalled();
    });

    it("gives its boxes the shared preview-box scroll hand-off", () => {
        const { container } = render(() => <OriginContainer node={node} collapsed={false} onToggle={() => {}} />);
        expect(container.querySelector(".agent-origin-body")!.classList.contains("scroll-handoff-box")).toBe(true);
    });
});

describe("OriginContainer jekt variant — peek tooltip", () => {
    it("shows time + estimate on hover", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <OriginContainer node={node} collapsed={true} onToggle={() => {}} />
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
                <OriginContainer node={node} collapsed={true} onToggle={() => {}} />
            ));
            hover(container);
            expect(document.body.querySelector(".agent-node-peek-overlay")).not.toBeNull();
            fireEvent.mouseLeave(container.querySelector(".agent-origin--agent") as HTMLElement);
            expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });
});

// The jekt is the container's `agent` variant (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md §4.2, D6).
describe("OriginContainer jekt variant — one container, neutral, a trust problem shows", () => {
    it("is the one container, never the human's turn, labelled from the sender", () => {
        const { container } = render(() => <OriginContainer node={node} collapsed={true} onToggle={() => {}} />);
        const root = container.querySelector(".agent-origin")!;
        expect(root.classList.contains("agent-origin--agent")).toBe(true);
        expect(root.querySelector(".agent-origin__label")?.textContent).toContain("Jekt from github-consumer");
        expect(container.querySelector(".agent-user-message")).toBeNull();
        expect(container.querySelector(".agent-jekt-bubble")).toBeNull();
    });

    it("an outgoing jekt reads 'to', and a collapsed one has no body", () => {
        const { container } = render(() => (
            <OriginContainer node={{ ...node, direction: "outgoing" }} collapsed={true} onToggle={() => {}} />
        ));
        expect(container.querySelector(".agent-origin__label")?.textContent).toContain("Jekt to naki");
        expect(container.querySelector(".agent-origin-body")).toBeNull();
    });

    it("warns on a sender nothing proved and on a sensitive tier, and only then", () => {
        const tags = (n: typeof node) =>
            [...render(() => <OriginContainer node={n} collapsed={true} onToggle={() => {}} />).container.querySelectorAll(".ui-tag")];
        const proven = tags({ ...node, trust: "lan-verified", tier: "coord" });
        expect(proven.some((t) => t.classList.contains("ui-tag--warn") || t.classList.contains("ui-tag--error"))).toBe(false);
        cleanup();
        expect(tags({ ...node, trust: "network-claimed" }).some((t) => t.classList.contains("ui-tag--warn"))).toBe(true);
        cleanup();
        const sensitive = render(() => <OriginContainer node={{ ...node, trust: "lan-verified", tier: "sensitive" }} collapsed={false} onToggle={() => {}} />).container;
        expect(sensitive.querySelector(".agent-origin--agent.tier-sensitive")).not.toBeNull();
        expect(sensitive.querySelector(".ui-tag--error")).not.toBeNull();
    });

    it("opens to the body, the sender line and the raw payload; a click on the body does not toggle", () => {
        const onToggle = vi.fn();
        const { container } = render(() => <OriginContainer node={{ ...node, heldForSecs: 600 }} collapsed={false} onToggle={onToggle} />);
        expect(container.querySelector(".agent-origin-body")?.textContent).toContain("PR #2676 reviewed");
        expect(container.querySelector(".agent-origin-meta")?.textContent).toContain("MSGID: inj-1");
        expect(container.querySelector(".agent-origin-meta")?.textContent).toContain("Held for 10 min");
        expect(container.querySelector(".agent-origin-raw")?.textContent).toContain("Raw payload");
        fireEvent.click(container.querySelector(".agent-origin-body")!);
        expect(onToggle).not.toHaveBeenCalled();
        fireEvent.click(container.querySelector(".agent-origin-summary")!);
        expect(onToggle).toHaveBeenCalledTimes(1);
    });
});
