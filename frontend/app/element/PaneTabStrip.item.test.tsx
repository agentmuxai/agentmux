// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What one pill does with what its strip gives it, in the paths the other PaneTabStrip.*.test files
 * leave open: the tooltip of a pill with a progress state or a seat (shown at once, hung below the
 * pill), the side a dragged pill would land on, which pill bounces when a tab lands, a drag that
 * starts not counting as a click, and a closing pill's close button.
 *
 * The drag adapter is replaced by a recorder (jsdom has no HTML5 drag), so the callbacks each pill
 * registers can be called directly, as PaneTabStrip.dropTarget.test.tsx does.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { endDrag } from "@/app/drag/drag-session";
import type { PaneTabProgress } from "@/app/store/pane-tab-progress";

interface DropTargetConfig {
    element: HTMLElement;
    onDrag: (args: unknown) => void;
    onDragLeave: () => void;
}
interface DraggableConfig {
    element: HTMLElement;
    onDragStart: () => void;
}
const dropTargets: DropTargetConfig[] = [];
const draggables: DraggableConfig[] = [];

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    draggable: (config: DraggableConfig) => {
        draggables.push(config);
        return () => {};
    },
    dropTargetForElements: (config: DropTargetConfig) => {
        dropTargets.push(config);
        return () => {};
    },
}));

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

// The pulse each pill plays when it lands: kept as the condition it was given, so a test can ask it.
const pulses: { el: () => Element | null | undefined; when: () => boolean }[] = [];
vi.mock("@/app/motion/solid", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/motion/solid")>()),
    usePulseWhen: (el: () => Element | null | undefined, when: () => boolean) => {
        pulses.push({ el, when });
    },
}));

import { computePosition } from "@floating-ui/dom";
import { markLanded } from "./pane-tab-landing";
import { PaneTabStrip } from "./PaneTabStrip";
import { resetTooltipTiming } from "./tooltip-timing";

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
};

const frame = () => new Promise((resolve) => setTimeout(resolve, 60));
const overlay = () => document.body.querySelector("[data-pane-overlay]") as HTMLElement | null;
const pill = (c: HTMLElement, id: string) => c.querySelector<HTMLElement>(`.pane-tab[data-tab-id="${id}"]`)!;
const tipOf = (c: HTMLElement, id: string) => pill(c, id).parentElement as HTMLElement;

beforeEach(() => {
    dropTargets.length = 0;
    draggables.length = 0;
    pulses.length = 0;
    endDrag("cancel");
    resetTooltipTiming();
    vi.mocked(computePosition).mockClear();
});
afterEach(() => cleanup());

describe("PaneTabStripItem — its tooltip", () => {
    it("names the progress state and shows at once for a pill that has one and nothing else to say", async () => {
        const working: PaneTabProgress = { state: "working", text: "Working" };
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                {...common}
                onActivate={vi.fn()}
                useProgress={(t: T) => (t.id === "b" ? () => working : () => null)}
            />
        ));
        fireEvent.mouseEnter(tipOf(container, "b"));
        await frame();

        const tip = overlay()!;
        expect(tip.querySelector(".pane-tab-tip-label")!.textContent).toBe("beta");
        expect(tip.querySelector(".pane-tab-tip-state")!.textContent).toBe("Working");
        expect(tip.firstElementChild!.getAttribute("style")).toContain("transition: none");
    });

    it("shows a pill with only a seat at once, with no fade", async () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                {...common}
                onActivate={vi.fn()}
                getTooltipSeat={(t: T) => (t.id === "b" ? "Specialist — reports to Camper" : undefined)}
            />
        ));
        fireEvent.mouseEnter(tipOf(container, "b"));
        await frame();

        expect(overlay()!.firstElementChild!.getAttribute("style")).toContain("transition: none");
    });

    it("hangs below the pill", async () => {
        const { container } = render(() => <PaneTabStrip tabs={TABS} activeId="a" {...common} onActivate={vi.fn()} />);
        fireEvent.mouseEnter(tipOf(container, "a"));
        await frame();

        const calls = vi.mocked(computePosition).mock.calls;
        expect(calls.length).toBeGreaterThan(0);
        expect(calls[calls.length - 1][2]).toMatchObject({ placement: "bottom" });
    });
});

describe("PaneTabStripItem — the side a dragged pill would land on", () => {
    function renderReorderable() {
        const result = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                {...common}
                onActivate={vi.fn()}
                paneKey="pane-1"
                onReorder={vi.fn()}
            />
        ));
        const target = dropTargets.find((d) => d.element.dataset.tabId === "b")!;
        // jsdom has no layout: the pill is 100 wide from x = 0.
        vi.spyOn(target.element, "getBoundingClientRect").mockReturnValue({ left: 0, width: 100 } as DOMRect);
        const over = (clientX: number) => target.onDrag({ location: { current: { input: { clientX } } } });
        const classes = () => ({
            before: pill(result.container, "b").classList.contains("pane-tab--drop-before"),
            after: pill(result.container, "b").classList.contains("pane-tab--drop-after"),
        });
        return { target, over, classes };
    }

    it("marks the before side over the left half, and only that", () => {
        const { over, classes } = renderReorderable();
        over(10);
        expect(classes()).toEqual({ before: true, after: false });
    });

    it("marks the after side over the right half, and only that", () => {
        const { over, classes } = renderReorderable();
        over(90);
        expect(classes()).toEqual({ before: false, after: true });
    });

    it("clears the mark when the drag leaves", () => {
        const { target, over, classes } = renderReorderable();
        over(10);
        target.onDragLeave();
        expect(classes()).toEqual({ before: false, after: false });
    });
});

describe("PaneTabStripItem — which pill bounces when a tab lands", () => {
    const bounces = (c: HTMLElement, id: string) => {
        const p = pulses.find((x) => x.el() === pill(c, id));
        expect(p, `a pulse registered for ${id}`).toBeDefined();
        return p!.when();
    };

    it("bounces the pill that landed in its own pane", () => {
        const { container } = render(() => (
            <PaneTabStrip tabs={TABS} activeId="a" {...common} onActivate={vi.fn()} paneKey="pane-1" />
        ));
        markLanded("b", "pane-1");
        expect(bounces(container, "b")).toBe(true);
    });

    it("does not bounce another pill of the same pane", () => {
        const { container } = render(() => (
            <PaneTabStrip tabs={TABS} activeId="a" {...common} onActivate={vi.fn()} paneKey="pane-1" />
        ));
        markLanded("b", "pane-1");
        expect(bounces(container, "a")).toBe(false);
    });

    it("does not bounce the pill of the same block in another pane", () => {
        const { container } = render(() => (
            <PaneTabStrip tabs={TABS} activeId="a" {...common} onActivate={vi.fn()} paneKey="pane-1" />
        ));
        markLanded("b", "pane-2");
        expect(bounces(container, "b")).toBe(false);
    });
});

describe("PaneTabStripItem — a drag that starts", () => {
    it("is not a click: the press that began it does not activate the pill", () => {
        const onActivate = vi.fn();
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                {...common}
                onActivate={onActivate}
                paneKey="pane-1"
                onReorder={vi.fn()}
            />
        ));
        const b = pill(container, "b");
        const drag = draggables.find((d) => d.element === b)!;

        fireEvent.mouseDown(b, { button: 0, clientX: 5, clientY: 5 });
        drag.onDragStart();
        fireEvent.click(b, { button: 0, clientX: 5, clientY: 5 });

        expect(onActivate).not.toHaveBeenCalled();
    });

    it("a press with no drag does activate it", () => {
        const onActivate = vi.fn();
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                {...common}
                onActivate={onActivate}
                paneKey="pane-1"
                onReorder={vi.fn()}
            />
        ));
        const b = pill(container, "b");

        fireEvent.mouseDown(b, { button: 0, clientX: 5, clientY: 5 });
        fireEvent.click(b, { button: 0, clientX: 5, clientY: 5 });

        expect(onActivate).toHaveBeenCalledWith("b");
    });
});

describe("PaneTabStripItem — a closing pill", () => {
    it("has its close button disabled, which closes nothing", () => {
        const onClose = vi.fn();
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                {...common}
                onActivate={vi.fn()}
                onClose={onClose}
                isClosing={(t: T) => t.id === "b"}
            />
        ));
        const button = pill(container, "b").querySelector<HTMLButtonElement>(".pane-tab-close")!;
        expect(button.disabled).toBe(true);

        fireEvent.click(button);

        expect(onClose).not.toHaveBeenCalled();
    });
});
