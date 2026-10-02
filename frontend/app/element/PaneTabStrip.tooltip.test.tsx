// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane tab pill's tooltip for an agent: its name and its summary (the same line
 * the Swarm row shows), shown immediately on hover. A tab with no summary keeps
 * exactly the tooltip it always had.
 */
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PaneTabStrip } from "./PaneTabStrip";

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(() => vi.fn()),
    computePosition: vi.fn(() => Promise.resolve({ x: 0, y: 0 })),
    flip: vi.fn(() => ({})),
    offset: vi.fn(() => ({})),
    shift: vi.fn(() => ({})),
}));

interface T {
    id: string;
    label: string;
}

const TABS: T[] = [
    { id: "agent", label: "AgentX" },
    { id: "term", label: "Terminal" },
];

const frame = () => new Promise((resolve) => setTimeout(resolve, 60));
const overlay = () => document.body.querySelector("[data-pane-overlay]") as HTMLElement | null;

afterEach(() => cleanup());

function renderStrip(detail: (t: T) => string | undefined) {
    return render(() => (
        <PaneTabStrip
            tabs={TABS}
            activeId="agent"
            getId={(t: T) => t.id}
            getLabel={(t: T) => t.label}
            getTooltipDetail={detail}
            onActivate={vi.fn()}
        />
    ));
}

describe("PaneTabStrip tooltip detail", () => {
    it("shows the name and the summary, visible one frame after hover with no fade", async () => {
        const { container } = renderStrip((t) => (t.id === "agent" ? "Develop hardening spec for swarm summaries" : undefined));
        const pill = container.querySelectorAll(".pane-tab-tip")[0] as HTMLElement;
        fireEvent.mouseEnter(pill);
        await frame();
        const tip = overlay()!;
        expect(tip.querySelector(".pane-tab-tip-label")!.textContent).toBe("AgentX");
        expect(tip.querySelector(".pane-tab-tip-summary")!.textContent).toBe("Develop hardening spec for swarm summaries");
        expect(tip.getAttribute("style")).toContain("opacity: 1");
        expect(tip.getAttribute("style")).toContain("transition: none");
    });

    it("leaves a tab with no summary exactly as it was: just the label, after the usual delay", async () => {
        const { container } = renderStrip((t) => (t.id === "agent" ? "Fixing login" : undefined));
        const pill = container.querySelectorAll(".pane-tab-tip")[1] as HTMLElement;
        fireEvent.mouseEnter(pill);
        await frame();
        const tip = overlay()!;
        expect(tip.textContent).toBe("Terminal");
        expect(tip.querySelector(".pane-tab-tip-summary")).toBeNull();
        // The default 300 ms show delay has not elapsed after 60 ms.
        expect(tip.getAttribute("style")).toContain("opacity: 0");
    });

    it("goes away the moment the pointer leaves", async () => {
        const { container } = renderStrip(() => "Fixing login");
        const pill = container.querySelectorAll(".pane-tab-tip")[0] as HTMLElement;
        fireEvent.mouseEnter(pill);
        await frame();
        expect(overlay()).not.toBeNull();
        fireEvent.mouseLeave(pill);
        await new Promise((resolve) => setTimeout(resolve, 0));
        expect(overlay()).toBeNull();
    });

    it("follows a summary that changes while the tooltip is open", async () => {
        const [summary, setSummary] = createSignal<string | undefined>("Fixing login");
        const { container } = renderStrip((t) => (t.id === "agent" ? summary() : undefined));
        const pill = container.querySelectorAll(".pane-tab-tip")[0] as HTMLElement;
        fireEvent.mouseEnter(pill);
        await frame();
        expect(overlay()!.querySelector(".pane-tab-tip-summary")!.textContent).toBe("Fixing login");
        setSummary("Reviewing the pull request");
        await new Promise((resolve) => setTimeout(resolve, 0));
        expect(overlay()!.querySelector(".pane-tab-tip-summary")!.textContent).toBe("Reviewing the pull request");
    });
});
