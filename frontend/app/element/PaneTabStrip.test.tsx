// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for PaneTabStrip — the shared, pane-type-agnostic tab strip.
 * Spec: docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §3.1.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { PaneTabStrip } from "./PaneTabStrip";

afterEach(() => cleanup());

interface T {
    id: string;
    label: string;
    dirty?: boolean;
}

const TABS: T[] = [
    { id: "a", label: "alpha" },
    { id: "b", label: "beta", dirty: true },
];

describe("PaneTabStrip", () => {
    it("renders one tab per item via accessor props", () => {
        render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
            />
        ));
        expect(screen.getByText("alpha")).toBeInTheDocument();
        expect(screen.getByText("beta")).toBeInTheDocument();
    });

    it("marks the active tab", () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="b"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
            />
        ));
        const tabs = container.querySelectorAll(".pane-tab");
        expect(tabs[0].classList.contains("pane-tab--active")).toBe(false);
        expect(tabs[1].classList.contains("pane-tab--active")).toBe(true);
    });

    it("fires onActivate when a non-active tab is clicked, not when the active tab is re-clicked", async () => {
        const onActivate = vi.fn();
        render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={onActivate}
            />
        ));
        const user = userEvent.setup();
        await user.click(screen.getByText("alpha"));
        expect(onActivate).not.toHaveBeenCalled();
        await user.click(screen.getByText("beta"));
        expect(onActivate).toHaveBeenCalledWith("b");
        expect(onActivate).toHaveBeenCalledTimes(1);
    });

    it("fires onClose when the close button is clicked, without also activating", async () => {
        const onActivate = vi.fn();
        const onClose = vi.fn();
        render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={onActivate}
                onClose={onClose}
            />
        ));
        const user = userEvent.setup();
        await user.click(screen.getAllByRole("button", { name: "Close tab" })[1]);
        expect(onClose).toHaveBeenCalledWith("b");
        expect(onActivate).not.toHaveBeenCalled();
    });

    it("omits the close button entirely when onClose is not provided", () => {
        render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
            />
        ));
        expect(screen.queryByRole("button", { name: "Close tab" })).toBeNull();
    });

    it("marks attention tabs via getAttention", () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                getAttention={(t: T) => !!t.dirty}
                onActivate={vi.fn()}
            />
        ));
        const tabs = container.querySelectorAll(".pane-tab");
        expect(tabs[0].classList.contains("pane-tab--attention")).toBe(false);
        expect(tabs[1].classList.contains("pane-tab--attention")).toBe(true);
    });

    it("applies extra classes from getTabClass", () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                getTabClass={(t: T) => ({ "custom-preview": t.id === "a" })}
                onActivate={vi.fn()}
            />
        ));
        const tabs = container.querySelectorAll(".pane-tab");
        expect(tabs[0].classList.contains("custom-preview")).toBe(true);
        expect(tabs[1].classList.contains("custom-preview")).toBe(false);
    });

    it("does not render the + button when onAdd is omitted", () => {
        render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
            />
        ));
        expect(screen.queryByRole("button", { name: "New tab" })).toBeNull();
    });

    it("renders the + button pinned last and fires onAdd", async () => {
        const onAdd = vi.fn();
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                onAdd={onAdd}
                addTitle="New shell tab"
            />
        ));
        const addBtn = screen.getByRole("button", { name: "New shell tab" });
        // .pane-tab-strip-inner (not .pane-tab-strip itself) owns tab/+
        // layout as of
        // docs/specs/SPEC_PANE_TAB_STRIP_CHROME_ZOOM_AND_SCROLL_CLEARANCE_2026_08_12.md
        // §A.2 — the outer box now only wraps that one inner layer.
        const inner = container.querySelector(".pane-tab-strip-inner");
        expect(inner?.lastElementChild).toBe(addBtn);
        const user = userEvent.setup();
        await user.click(addBtn);
        expect(onAdd).toHaveBeenCalledTimes(1);
    });

    it("renders custom label content from renderLabel instead of the plain label", () => {
        render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                renderLabel={(t: T) => <span data-testid={`custom-${t.id}`}>{t.label.toUpperCase()}</span>}
                onActivate={vi.fn()}
            />
        ));
        expect(screen.getByTestId("custom-a")).toHaveTextContent("ALPHA");
        expect(screen.queryByText("alpha")).toBeNull();
    });

    it("fires onTabDoubleClick with the tab item", async () => {
        const onDbl = vi.fn();
        render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                onTabDoubleClick={onDbl}
            />
        ));
        const user = userEvent.setup();
        await user.dblClick(screen.getByText("beta"));
        expect(onDbl).toHaveBeenCalledWith(TABS[1]);
    });
});

// SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md §2.4 / Codex's review of
// PR #2768: a plain CSS `transition: width` never fires for this box (width
// stays `auto` the whole time — only its DOM-content-driven USED size
// changes), so the width transition is implemented as a measured, JS-driven
// FLIP instead. `getBoundingClientRect` is mocked so jsdom's lack of real
// layout doesn't matter — only the values these tests hand it do.
describe("PaneTabStrip — animateWidth (SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md §2.4)", () => {
    let currentWidth = 100;

    beforeEach(() => {
        currentWidth = 100;
        vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
            () => ({ width: currentWidth }) as DOMRect
        );
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.restoreAllMocks();
        vi.useRealTimers();
    });

    it("does nothing when animateWidth is not passed (default false, e.g. the agent pane's full-width strip)", async () => {
        const [tabs, setTabs] = createSignal<T[]>([TABS[0]]);
        const { container } = render(() => (
            <PaneTabStrip
                tabs={tabs()}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
            />
        ));
        await Promise.resolve();
        const strip = container.querySelector(".pane-tab-strip") as HTMLElement;
        currentWidth = 200;
        setTabs(TABS);
        await Promise.resolve();
        expect(strip.style.width).toBe("");
    });

    it("holds the old measured width, then transitions to the new one on a tabs.length change", async () => {
        const [tabs, setTabs] = createSignal<T[]>([TABS[0]]);
        const { container } = render(() => (
            <PaneTabStrip
                tabs={tabs()}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                animateWidth
            />
        ));
        await Promise.resolve(); // flush the mount-time measurement (records 100, no animation)
        const strip = container.querySelector(".pane-tab-strip") as HTMLElement;
        expect(strip.style.width).toBe("");

        currentWidth = 180;
        setTabs(TABS); // tabs.length: 1 -> 2
        await Promise.resolve();
        expect(strip.style.width).toBe("180px");
        expect(strip.style.transition).toBe("width 160ms ease-out");
    });

    it("clears the inline width/transition after the transition duration elapses", async () => {
        const [tabs, setTabs] = createSignal<T[]>([TABS[0]]);
        const { container } = render(() => (
            <PaneTabStrip
                tabs={tabs()}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                animateWidth
            />
        ));
        await Promise.resolve();
        const strip = container.querySelector(".pane-tab-strip") as HTMLElement;

        currentWidth = 180;
        setTabs(TABS);
        await Promise.resolve();
        expect(strip.style.width).toBe("180px");

        vi.advanceTimersByTime(200);
        expect(strip.style.width).toBe("");
        expect(strip.style.transition).toBe("");
    });

    it("does not animate when the measured width is unchanged", async () => {
        const [tabs, setTabs] = createSignal<T[]>([TABS[0]]);
        const { container } = render(() => (
            <PaneTabStrip
                tabs={tabs()}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                animateWidth
            />
        ));
        await Promise.resolve();
        const strip = container.querySelector(".pane-tab-strip") as HTMLElement;

        // currentWidth stays 100 — a tabs.length change that happens not to
        // move the measured box (e.g. padding absorbed it).
        setTabs(TABS);
        await Promise.resolve();
        expect(strip.style.width).toBe("");
    });

    it("does not animate on the initial mount even if animateWidth is set", async () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                animateWidth
            />
        ));
        await Promise.resolve();
        const strip = container.querySelector(".pane-tab-strip") as HTMLElement;
        expect(strip.style.width).toBe("");
    });
});
