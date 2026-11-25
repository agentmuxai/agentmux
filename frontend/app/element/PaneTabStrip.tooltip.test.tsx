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
    // The real autoUpdate places the panel at once; AnchoredPopover places it only through it.
    autoUpdate: vi.fn((_reference: unknown, _floating: unknown, update: () => void) => {
        update();
        return vi.fn();
    }),
    computePosition: vi.fn(() => Promise.resolve({ x: 0, y: 0 })),
    flip: vi.fn(() => ({})),
    offset: vi.fn(() => ({})),
    shift: vi.fn(() => ({})),
    size: vi.fn(() => ({})),
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

function renderStrip(
    detail: (t: T) => string | undefined,
    seat?: (t: T) => string | undefined,
    extra: Partial<Parameters<typeof PaneTabStrip<T>>[0]> = {}
) {
    return render(() => (
        <PaneTabStrip
            tabs={TABS}
            activeId="agent"
            getId={(t: T) => t.id}
            getLabel={(t: T) => t.label}
            getTooltipDetail={detail}
            getTooltipSeat={seat}
            onActivate={vi.fn()}
            {...extra}
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
        expect(tip.firstElementChild!.getAttribute("style")).toContain("opacity: 1");
        expect(tip.firstElementChild!.getAttribute("style")).toContain("transition: none");
    });

    it("shows a tab with no summary just its label, and at once like every tab: the whole title of a tab cut to its start", async () => {
        const { container } = renderStrip((t) => (t.id === "agent" ? "Fixing login" : undefined));
        const pill = container.querySelectorAll(".pane-tab-tip")[1] as HTMLElement;
        fireEvent.mouseEnter(pill);
        await frame();
        const tip = overlay()!;
        expect(tip.textContent).toBe("Terminal");
        expect(tip.querySelector(".pane-tab-tip-summary")).toBeNull();
        // At once (no fade), whatever the tab: a tab cut to an icon is named the moment the pointer is on it.
        expect(tip.firstElementChild!.getAttribute("style")).toContain("transition: none");
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

    it("adds an agent's seat between the name and the summary (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4)", async () => {
        const seat = "Leader · product — reports to the operator — agent pane, identity";
        const { container } = renderStrip(
            (t) => (t.id === "agent" ? "Fixing login" : undefined),
            (t) => (t.id === "agent" ? seat : undefined)
        );
        fireEvent.mouseEnter(container.querySelectorAll(".pane-tab-tip")[0] as HTMLElement);
        await frame();
        const body = overlay()!.querySelector(".pane-tab-tip-body")!;
        expect([...body.children].map((c) => c.className)).toEqual([
            "pane-tab-tip-label",
            "pane-tab-tip-seat",
            "pane-tab-tip-summary",
        ]);
        expect(body.querySelector(".pane-tab-tip-seat")!.textContent).toBe(seat);
    });

    it("shows a seat with no summary, at once", async () => {
        const { container } = renderStrip(
            () => undefined,
            (t) => (t.id === "agent" ? "Specialist — reports to Camper" : undefined)
        );
        fireEvent.mouseEnter(container.querySelectorAll(".pane-tab-tip")[0] as HTMLElement);
        await frame();
        const tip = overlay()!;
        expect(tip.querySelector(".pane-tab-tip-seat")!.textContent).toBe("Specialist — reports to Camper");
        expect(tip.querySelector(".pane-tab-tip-summary")).toBeNull();
        expect(tip.firstElementChild!.getAttribute("style")).toContain("opacity: 1");
    });

    it("shows the harness and model line of an agent tab, at once, with nothing else to show", async () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="agent"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                getTooltipModel={(t: T) => (t.id === "agent" ? "Claude Code harness running on DeepSeek V4.1 Flash via OpenRouter" : undefined)}
                onActivate={vi.fn()}
            />
        ));
        fireEvent.mouseEnter(container.querySelectorAll(".pane-tab-tip")[0] as HTMLElement);
        await frame();
        const tip = overlay()!;
        expect(tip.querySelector(".pane-tab-tip-label")!.textContent).toBe("AgentX");
        expect(tip.querySelector(".pane-tab-tip-seat")!.textContent).toBe(
            "Claude Code harness running on DeepSeek V4.1 Flash via OpenRouter"
        );
        expect(tip.firstElementChild!.getAttribute("style")).toContain("opacity: 1");
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

describe("an agent's hover card", () => {
    const agentOnly = (value: string) => (t: T) => (t.id === "agent" ? value : undefined);

    it("says its name, its state with a dot in the Swarm's colour, where it runs, its status line and how old that is", async () => {
        const { container } = renderStrip(agentOnly("Fixing login"), agentOnly("Worker · reports to AgentY"), {
            getTooltipWhere: agentOnly("In a container · On this computer"),
            getTooltipAge: agentOnly("12 min ago"),
            getTooltipModel: agentOnly("Claude · Opus"),
            useProgress: (t: T) => () => (t.id === "agent" ? { state: "working" as const, text: "Working" } : null),
        });
        fireEvent.mouseEnter(container.querySelectorAll(".pane-tab-tip")[0] as HTMLElement);
        await frame();
        const tip = overlay()!;
        expect(tip.querySelector(".pane-tab-tip-label")!.textContent).toBe("AgentX");
        const state = tip.querySelector("[data-testid=pane-tab-tip-state]")!;
        expect(state.textContent).toBe("Working");
        expect(state.getAttribute("data-state")).toBe("working");
        expect(state.querySelector(".pane-tab-tip-dot")).not.toBeNull();
        expect(tip.querySelector("[data-testid=pane-tab-tip-where]")!.textContent).toBe("In a container · On this computer");
        expect(tip.querySelector(".pane-tab-tip-summary")!.textContent).toContain("Fixing login");
        expect(tip.querySelector("[data-testid=pane-tab-tip-age]")!.textContent).toContain("12 min ago");
        expect(tip.textContent).toContain("Claude · Opus");
        expect(tip.textContent).toContain("Worker · reports to AgentY");
    });

    it("says the task the agent holds and how full its context is, and nothing for either until it has them", async () => {
        const { container } = renderStrip(agentOnly("Fixing login"), undefined, {
            getTooltipWhere: agentOnly("On this computer"),
            getTooltipTask: agentOnly("#48669 Pane tabs: no extra buttons"),
            getTooltipContext: agentOnly("41% of context"),
        });
        fireEvent.mouseEnter(container.querySelectorAll(".pane-tab-tip")[0] as HTMLElement);
        await frame();
        const tip = overlay()!;
        expect(tip.querySelector("[data-testid=pane-tab-tip-task]")!.textContent).toBe("#48669 Pane tabs: no extra buttons");
        expect(tip.querySelector("[data-testid=pane-tab-tip-context]")!.textContent).toBe("41% of context");
        // Text, not a link: the card never takes the pointer.
        expect(tip.querySelector("a, button")).toBeNull();
        cleanup();
        const bare = renderStrip(agentOnly("Fixing login"), undefined, { getTooltipWhere: agentOnly("On this computer") });
        fireEvent.mouseEnter(bare.container.querySelectorAll(".pane-tab-tip")[0] as HTMLElement);
        await frame();
        expect(overlay()!.querySelector("[data-testid=pane-tab-tip-task]")).toBeNull();
        expect(overlay()!.querySelector("[data-testid=pane-tab-tip-context]")).toBeNull();
    });

    it("calls an agent with no turn running idle, in the Swarm's idle colour", async () => {
        const { container } = renderStrip(() => undefined, undefined, {
            getTooltipWhere: agentOnly("On this computer"),
            getTooltipIdle: () => true,
        });
        fireEvent.mouseEnter(container.querySelectorAll(".pane-tab-tip")[0] as HTMLElement);
        await frame();
        const state = overlay()!.querySelector("[data-testid=pane-tab-tip-state]")!;
        expect(state.textContent).toBe("Idle");
        expect(state.getAttribute("data-state")).toBe("idle");
    });

    it("does not call an agent idle when it cannot start a turn (a usage limit, a refused sign-in)", async () => {
        const { container } = renderStrip(() => undefined, undefined, {
            getTooltipWhere: agentOnly("On this computer"),
            getTooltipIdle: () => false,
        });
        fireEvent.mouseEnter(container.querySelectorAll(".pane-tab-tip")[0] as HTMLElement);
        await frame();
        expect(overlay()!.querySelector("[data-testid=pane-tab-tip-state]")).toBeNull();
        expect(overlay()!.textContent).toContain("On this computer");
    });

    it("has no age when the line is not the agent's own, and no state line for a tab that is not an agent's", async () => {
        const { container } = renderStrip(agentOnly("Generated title"), undefined, { getTooltipWhere: agentOnly("On this computer") });
        fireEvent.mouseEnter(container.querySelectorAll(".pane-tab-tip")[0] as HTMLElement);
        await frame();
        expect(overlay()!.querySelector("[data-testid=pane-tab-tip-age]")).toBeNull();
        cleanup();
        const plain = renderStrip(() => undefined);
        fireEvent.mouseEnter(plain.container.querySelectorAll(".pane-tab-tip")[1] as HTMLElement);
        await frame();
        expect(overlay()!.querySelector("[data-testid=pane-tab-tip-state]")).toBeNull();
        expect(overlay()!.textContent).toBe("Terminal");
    });
});
