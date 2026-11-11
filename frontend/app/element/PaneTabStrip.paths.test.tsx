// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Paths through PaneTabStrip itself (the strip component, not its pills or hooks) that the other
 * PaneTabStrip.*.test files leave open: what it forwards to each pill, the flush-left fade and when
 * it is measured again, the landing cue of a document strip when its selected tab closes, the
 * hover classes on the strip box, a double-click staying inside it, and the `+` button.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { WindowTabDisplayedProvider } from "@/app/workspace/window-tab-visibility";
import { beginHover, resetFlashBudget } from "./drop-blink-clock";
import { landedTab } from "./pane-tab-landing";
import { resetTooltipTiming } from "./tooltip-timing";

const dropTargetCalls: { onDragEnter: (args?: unknown) => void; onDragLeave: () => void }[] = [];

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    draggable: () => () => {},
    dropTargetForElements: (config: { onDragEnter: (args?: unknown) => void; onDragLeave: () => void }) => {
        dropTargetCalls.push(config);
        return () => {};
    },
}));

// Which motion categories run their reduced treatment, set per test.
let reduced: string[] = [];
vi.mock("@/app/motion/motion", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/motion/motion")>()),
    shouldReduceMotion: (category: string) => reduced.includes(category),
}));

import { PaneTabStrip } from "./PaneTabStrip";

interface T {
    id: string;
    label: string;
}

const TABS: T[] = [
    { id: "a", label: "alpha" },
    { id: "b", label: "beta" },
];

const common = {
    getId: (t: T) => t.id,
    getLabel: (t: T) => t.label,
    onActivate: () => {},
};

const frame = () => new Promise((r) => requestAnimationFrame(() => r(null)));

beforeEach(() => {
    dropTargetCalls.length = 0;
    reduced = [];
    resetFlashBudget();
    resetTooltipTiming();
});
afterEach(() => cleanup());

describe("PaneTabStrip — what each pill is given", () => {
    it("shows getTooltip's text as the pill's tooltip, in place of its label", () => {
        const { container } = render(() => (
            <PaneTabStrip tabs={TABS} activeId="a" {...common} getTooltip={(t: T) => `${t.label} (full name)`} />
        ));
        const tip = container.querySelectorAll<HTMLElement>(".pane-tab-tip")[1];
        fireEvent.mouseEnter(tip);
        const overlay = document.body.querySelector("[data-pane-overlay]")!;
        expect(overlay.textContent).toBe("beta (full name)");
    });

    it("hands a right-click to onTabContextMenu with that pill's tab and the event", () => {
        const onTabContextMenu = vi.fn();
        const { container } = render(() => (
            <PaneTabStrip tabs={TABS} activeId="a" {...common} onTabContextMenu={onTabContextMenu} />
        ));
        fireEvent.contextMenu(container.querySelector<HTMLElement>('[data-tab-id="b"]')!);
        expect(onTabContextMenu).toHaveBeenCalledTimes(1);
        expect(onTabContextMenu.mock.calls[0][0]).toBe(TABS[1]);
        expect(onTabContextMenu.mock.calls[0][1]).toBeInstanceOf(MouseEvent);
    });
});

describe("PaneTabStrip — the fade at the left edge", () => {
    // The strip is scrolled (scrollLeft > 1), so the selected tab matters: flush against the left
    // edge it drops the left fade. jsdom has no layout, so each tab's left edge is set per test.
    let tabLeft: Record<string, number> = {};
    let rectSpy: ReturnType<typeof vi.spyOn>;
    beforeEach(() => {
        tabLeft = { a: 0, b: 200 };
        const geom = { scrollLeft: 50, clientWidth: 100, scrollWidth: 300 };
        for (const k of ["scrollLeft", "clientWidth", "scrollWidth"] as const) {
            Object.defineProperty(HTMLElement.prototype, k, {
                configurable: true,
                get() {
                    return geom[k];
                },
                set() {},
            });
        }
        rectSpy = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
            this: HTMLElement
        ) {
            const id = this.dataset.tabId;
            const left = id ? (tabLeft[id] ?? 0) : 0;
            const width = id ? 50 : 100;
            return { left, right: left + width, width, top: 0, bottom: 20, height: 20, x: left, y: 0 } as DOMRect;
        });
    });
    afterEach(() => {
        rectSpy.mockRestore();
        for (const k of ["scrollLeft", "clientWidth", "scrollWidth"] as const) {
            delete (HTMLElement.prototype as unknown as Record<string, unknown>)[k];
        }
    });

    const FLUSH = "pane-tab-strip--flush-left";
    const strip = (container: HTMLElement) => container.querySelector(".pane-tab-strip")!;

    it("drops the left fade when the selected tab is the one at the left edge", () => {
        const { container } = render(() => <PaneTabStrip tabs={TABS} activeId="a" {...common} />);
        expect(strip(container).classList.contains(FLUSH)).toBe(true);
    });

    it("keeps it when another tab is at the left edge and the selected one is far to the right", () => {
        const { container } = render(() => <PaneTabStrip tabs={TABS} activeId="b" {...common} />);
        expect(strip(container).classList.contains(FLUSH)).toBe(false);
    });

    it("measures again when the selection changes, without a scroll", async () => {
        const [active, setActive] = createSignal("a");
        const { container } = render(() => <PaneTabStrip tabs={TABS} activeId={active()} {...common} />);
        expect(strip(container).classList.contains(FLUSH)).toBe(true);
        setActive("b");
        await frame();
        expect(strip(container).classList.contains(FLUSH)).toBe(false);
        setActive("a");
        await frame();
        expect(strip(container).classList.contains(FLUSH)).toBe(true);
    });

    it("measures again when a hidden window tab is shown, not while it is hidden", async () => {
        const [displayed, setDisplayed] = createSignal(true);
        const { container } = render(() => (
            <WindowTabDisplayedProvider value={displayed}>
                <PaneTabStrip tabs={TABS} activeId="a" {...common} />
            </WindowTabDisplayedProvider>
        ));
        expect(strip(container).classList.contains(FLUSH)).toBe(true);
        setDisplayed(false);
        tabLeft.a = 200;
        await frame();
        expect(strip(container).classList.contains(FLUSH)).toBe(true);
        setDisplayed(true);
        await frame();
        expect(strip(container).classList.contains(FLUSH)).toBe(false);
    });

    it("measures once on mount, not again a frame later", async () => {
        const { container } = render(() => <PaneTabStrip tabs={TABS} activeId="a" {...common} />);
        expect(strip(container).classList.contains(FLUSH)).toBe(true);
        tabLeft.a = 200;
        await frame();
        await frame();
        expect(strip(container).classList.contains(FLUSH)).toBe(true);
    });
});

describe("PaneTabStrip — the landing cue of a document strip", () => {
    const doc = { docType: "editor", blockId: "blk-1", onReorder: () => true } as const;
    const THREE: T[] = [...TABS, { id: "c", label: "gamma" }];

    beforeEach(() => vi.useFakeTimers());
    afterEach(() => {
        vi.runOnlyPendingTimers();
        vi.useRealTimers();
    });

    it("lands the tab that takes the place of the selected one when that one closes", () => {
        const [tabs, setTabs] = createSignal(THREE);
        const [active, setActive] = createSignal("b");
        render(() => <PaneTabStrip tabs={tabs()} activeId={active()} {...common} docDrag={doc} />);
        expect(landedTab()).toBeNull();
        setTabs([THREE[0], THREE[2]]);
        setActive("c");
        expect(landedTab()?.ids).toEqual(new Set(["c"]));
        expect(landedTab()?.paneKey).toBe("blk-1");
    });

    it("lands nothing when the feedback category runs its reduced treatment", () => {
        reduced = ["feedback"];
        const [tabs, setTabs] = createSignal(THREE);
        const [active, setActive] = createSignal("b");
        render(() => <PaneTabStrip tabs={tabs()} activeId={active()} {...common} docDrag={doc} />);
        setTabs([THREE[0], THREE[2]]);
        setActive("c");
        expect(landedTab()).toBeNull();
    });

    it("still lands when only other categories are reduced", () => {
        reduced = ["status", "transition", "decorative"];
        const [tabs, setTabs] = createSignal(THREE);
        const [active, setActive] = createSignal("b");
        render(() => <PaneTabStrip tabs={tabs()} activeId={active()} {...common} docDrag={doc} />);
        setTabs([THREE[0], THREE[2]]);
        setActive("c");
        expect(landedTab()?.ids).toEqual(new Set(["c"]));
    });

    it("lands nothing when no tab is selected after the close", () => {
        const [tabs, setTabs] = createSignal(THREE);
        const [active, setActive] = createSignal<string | null>("b");
        render(() => <PaneTabStrip tabs={tabs()} activeId={active()} {...common} docDrag={doc} />);
        setTabs([THREE[0], THREE[2]]);
        setActive(null);
        expect(landedTab()).toBeNull();
    });

    it("lands nothing when a tab becomes selected where none was", () => {
        const [active, setActive] = createSignal<string | null>(null);
        render(() => <PaneTabStrip tabs={TABS} activeId={active()} {...common} docDrag={doc} />);
        setActive("a");
        expect(landedTab()).toBeNull();
    });

    it("lands nothing when the tabs change but the selection stays put, even on an id that is not in them", () => {
        const [tabs, setTabs] = createSignal(THREE);
        render(() => <PaneTabStrip tabs={tabs()} activeId="gone" {...common} docDrag={doc} />);
        setTabs([THREE[0], THREE[2]]);
        expect(landedTab()).toBeNull();
    });

    it("lands nothing when the selection moves to another tab and the old one is still there", () => {
        const [active, setActive] = createSignal("a");
        render(() => <PaneTabStrip tabs={TABS} activeId={active()} {...common} docDrag={doc} />);
        setActive("b");
        expect(landedTab()).toBeNull();
    });

    it("does not do it for a pane's own strip, only a document strip", () => {
        const [tabs, setTabs] = createSignal(THREE);
        const [active, setActive] = createSignal("b");
        render(() => <PaneTabStrip tabs={tabs()} activeId={active()} {...common} />);
        setTabs([THREE[0], THREE[2]]);
        setActive("c");
        expect(landedTab()).toBeNull();
    });
});

describe("PaneTabStrip — the hover classes on the strip box", () => {
    const FOREIGN = "pane-tab-strip--foreign-hover";
    const NO_BLINK = "pane-tab-strip--no-blink";
    const NO_INVERT = "pane-tab-strip--no-invert";
    const classes = (el: Element) => [FOREIGN, NO_BLINK, NO_INVERT].filter((c) => el.classList.contains(c));

    const standalone = () => {
        const { container } = render(() => (
            <PaneTabStrip tabs={TABS} activeId="a" {...common} paneKey="pane-self" onReceiveForeignTab={vi.fn()} />
        ));
        return container.querySelector(".pane-tab-strip")!;
    };
    const inHeader = () => {
        const { container } = render(() => (
            <div data-role="block-header">
                <PaneTabStrip tabs={TABS} activeId="a" {...common} paneKey="pane-self" onReceiveForeignTab={vi.fn()} />
            </div>
        ));
        return container.querySelector(".pane-tab-strip")!;
    };
    // The window's flash budget decides how a hover flashes: a hover left running a while ago leaves
    // room for the held negative alone ("steady"), a fresh blink leaves none ("plain").
    const steadyBudget = () => beginHover(performance.now() - 1450);
    const plainBudget = () => beginHover(performance.now());

    it("a hover that blinks outlines the strip, with neither of the quieter treatments", async () => {
        const el = standalone();
        dropTargetCalls[0].onDragEnter();
        await Promise.resolve();
        expect(classes(el)).toEqual([FOREIGN]);
    });

    it("a hover that holds the negative without blinking adds no-blink", async () => {
        steadyBudget();
        const el = standalone();
        dropTargetCalls[0].onDragEnter();
        await Promise.resolve();
        expect(classes(el)).toEqual([FOREIGN, NO_BLINK]);
    });

    it("a hover with no flash budget left adds no-invert", async () => {
        plainBudget();
        const el = standalone();
        dropTargetCalls[0].onDragEnter();
        await Promise.resolve();
        expect(classes(el)).toEqual([FOREIGN, NO_INVERT]);
    });

    it.each([
        ["steady", steadyBudget],
        ["plain", plainBudget],
    ])("takes every hover class off when a %s hover ends", async (_kind, budget) => {
        budget();
        const el = standalone();
        dropTargetCalls[0].onDragEnter();
        await Promise.resolve();
        expect(classes(el).length).toBeGreaterThan(1);
        dropTargetCalls[0].onDragLeave();
        await Promise.resolve();
        expect(classes(el)).toEqual([]);
    });

    it.each([
        ["blink", () => {}],
        ["steady", steadyBudget],
        ["plain", plainBudget],
    ])("puts none of them on the strip when the header row takes the highlight (%s)", async (_kind, budget) => {
        budget();
        const el = inHeader();
        dropTargetCalls[0].onDragEnter();
        await Promise.resolve();
        expect(classes(el)).toEqual([]);
    });
});

describe("PaneTabStrip — a double-click inside the strip", () => {
    it("does not reach what the strip sits in", () => {
        const outer = vi.fn();
        const { container } = render(() => (
            <div class="outer" onDblClick={outer}>
                <PaneTabStrip tabs={TABS} activeId="a" {...common} />
            </div>
        ));
        fireEvent.dblClick(container.querySelector(".pane-tab-strip")!);
        expect(outer).not.toHaveBeenCalled();
        // The same event on the wrapper itself is seen, so the handler is wired and the strip stops it.
        fireEvent.dblClick(container.querySelector(".outer")!);
        expect(outer).toHaveBeenCalledTimes(1);
    });
});

describe("PaneTabStrip — the + button", () => {
    const withAdd = (extra: { addTitle?: string; addLabel?: string }, onAdd = vi.fn()) =>
        render(() => <PaneTabStrip tabs={TABS} activeId="a" {...common} onAdd={onAdd} {...extra} />);

    it("is named New tab, and says so in its tooltip, when the caller names it nothing else", () => {
        const { container } = withAdd({});
        expect(screen.getByRole("button", { name: "New tab" })).toBeInTheDocument();
        fireEvent.mouseEnter(container.querySelector(".pane-tab-strip-add-tip")!);
        expect(document.body.querySelector("[data-pane-overlay]")!.textContent).toBe("New tab");
    });

    it("uses addTitle for its tooltip", () => {
        const { container } = withAdd({ addTitle: "New shell tab" });
        fireEvent.mouseEnter(container.querySelector(".pane-tab-strip-add-tip")!);
        expect(document.body.querySelector("[data-pane-overlay]")!.textContent).toBe("New shell tab");
    });

    it("is named by addTitle when it has no label, and by addLabel when it has one", () => {
        withAdd({ addTitle: "New shell tab" });
        expect(screen.getByRole("button", { name: "New shell tab" })).toBeInTheDocument();
        cleanup();
        withAdd({ addTitle: "New shell tab", addLabel: "New Agent" });
        expect(screen.getByRole("button", { name: "New Agent" })).toBeInTheDocument();
        expect(screen.getByText("New Agent")).toHaveClass("pane-tab-strip-add-label");
    });

    it("hands the click itself to onAdd, so a caller can place a picker at it", () => {
        const onAdd = vi.fn();
        withAdd({}, onAdd);
        fireEvent.click(screen.getByRole("button", { name: "New tab" }), { clientX: 12, clientY: 34 });
        expect(onAdd).toHaveBeenCalledTimes(1);
        const e = onAdd.mock.calls[0][0] as MouseEvent;
        expect(e).toBeInstanceOf(MouseEvent);
        expect([e.type, e.clientX, e.clientY]).toEqual(["click", 12, 34]);
    });

    it("shows its tooltip as soon as the panel is placed, not after the default delay", async () => {
        vi.useFakeTimers();
        try {
            const { container } = withAdd({ addTitle: "New shell tab" });
            fireEvent.mouseEnter(container.querySelector(".pane-tab-strip-add-tip")!);
            await vi.advanceTimersByTimeAsync(20);
            const tip = document.body.querySelector("[data-pane-overlay]") as HTMLElement;
            expect(tip.firstElementChild!.getAttribute("style")).toContain("opacity: 1");
        } finally {
            vi.useRealTimers();
        }
    });
});
