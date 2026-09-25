// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for PaneTabStrip — the shared, pane-type-agnostic tab strip.
 * Spec: docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §3.1.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { emitActivityFlash } from "@/app/notification/activity-flash";
import { dropPositionForPointerX, foreignDropHighlightFor, foreignDropRootFor, PaneTabStrip } from "./PaneTabStrip";

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

    it("sets --pane-tab-bg/--pane-tab-underline from getColor, per tab, omitting whichever half is absent", () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                getColor={(t: T) =>
                    t.id === "a" ? { background: "#112233", underline: "#445566" } : { background: "#778899" }
                }
                onActivate={vi.fn()}
            />
        ));
        const tabs = container.querySelectorAll<HTMLElement>(".pane-tab");
        expect(tabs[0].style.getPropertyValue("--pane-tab-bg")).toBe("#112233");
        expect(tabs[0].style.getPropertyValue("--pane-tab-underline")).toBe("#445566");
        expect(tabs[1].style.getPropertyValue("--pane-tab-bg")).toBe("#778899");
        expect(tabs[1].style.getPropertyValue("--pane-tab-underline")).toBe("");
    });

    it("keeps --pane-tab-bg and --colored on the ACTIVE tab, so it can paint its own color rather than the plain surface", () => {
        // The SCSS precondition for SPEC_PANE_HEADER_TAIL_COLOR's follow-up
        // fix: `.pane-tab--active` resolves `var(--pane-tab-bg,
        // var(--block-bg-color))`, which only works if the active tab still
        // carries its own color inline. Reported as "the selected tab is
        // assuming the color of the tail" — before that fix, `--active`
        // overrode the background unconditionally, so every INACTIVE pill
        // showed its hue while the selected one went neutral. jsdom
        // resolves no stylesheet, so this pins the JS half; the computed
        // background itself is verified live (see the spec's §6).
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                getColor={() => ({ background: "#112233", underline: "#445566", neutralBackground: "#101010" })}
                onActivate={vi.fn()}
            />
        ));
        const active = container.querySelectorAll<HTMLElement>(".pane-tab")[0];
        expect(active.classList.contains("pane-tab--active")).toBe(true);
        expect(active.style.getPropertyValue("--pane-tab-bg")).toBe("#112233");
        expect(active.classList.contains("pane-tab--colored")).toBe(true);
    });

    it("sets --pane-tab-neutral-bg for a tab with only a neutralBackground, without marking it colored", () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                getColor={() => ({ neutralBackground: "#101010" })}
                onActivate={vi.fn()}
            />
        ));
        const tab = container.querySelectorAll<HTMLElement>(".pane-tab")[1];
        expect(tab.style.getPropertyValue("--pane-tab-neutral-bg")).toBe("#101010");
        expect(tab.style.getPropertyValue("--pane-tab-bg")).toBe("");
        expect(tab.classList.contains("pane-tab--colored")).toBe(false);
    });

    it("sets neither custom property when getColor is omitted or returns undefined for a tab", () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                getColor={() => undefined}
                onActivate={vi.fn()}
            />
        ));
        const tabs = container.querySelectorAll<HTMLElement>(".pane-tab");
        for (const tab of tabs) {
            expect(tab.style.getPropertyValue("--pane-tab-bg")).toBe("");
            expect(tab.style.getPropertyValue("--pane-tab-underline")).toBe("");
        }
    });

    it("applies pane-tab--colored (gates PaneTabStrip.scss's lighter-on-hover rule) only for a tab with its own background", () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                getColor={(t: T) => (t.id === "a" ? { background: "#112233" } : undefined)}
                onActivate={vi.fn()}
            />
        ));
        const tabs = container.querySelectorAll<HTMLElement>(".pane-tab");
        expect(tabs[0].classList.contains("pane-tab--colored")).toBe(true);
        expect(tabs[1].classList.contains("pane-tab--colored")).toBe(false);
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
        // The button itself is wrapped in the Tooltip's own div
        // (.pane-tab-strip-add-tip, instant delayMs={0} — same reasoning as
        // the per-tab Tooltip above it), so the flex-last child is that
        // wrapper, not addBtn directly; assert it CONTAINS addBtn instead.
        const inner = container.querySelector(".pane-tab-strip-inner");
        expect(inner?.lastElementChild).toContainElement(addBtn);
        const user = userEvent.setup();
        await user.click(addBtn);
        expect(onAdd).toHaveBeenCalledTimes(1);
    });

    // Native `title` is slow/inconsistent in CEF (same reasoning as the
    // per-tab Tooltip) — the + button's tooltip uses the Portal-based
    // Tooltip component with delayMs={0} instead, so it shows essentially
    // immediately rather than after the component's own 300ms default.
    //
    // reagent P2 on PR #2975: the first cut of this test only checked that
    // the tooltip's text existed in document.body — but tooltip.tsx mounts
    // that div (`isOpen`) synchronously on hover regardless of `delayMs`;
    // only its OPACITY (`isVisible`, gated behind a `setTimeout(fn,
    // delayMs())`) actually depends on the delay. That version would have
    // passed identically even with the 300ms default, so it verified
    // nothing. Asserting on the floating div's own `opacity` (via its
    // `data-pane-overlay` marker, tooltip.tsx's own hook for this exact
    // node) is what actually distinguishes delayMs={0} from the default.
    it("shows the + button's tooltip almost immediately (delayMs={0}) — opacity flips well under the 300ms default", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <PaneTabStrip
                    tabs={TABS}
                    activeId="a"
                    getId={(t: T) => t.id}
                    getLabel={(t: T) => t.label}
                    onActivate={vi.fn()}
                    onAdd={vi.fn()}
                    addTitle="New shell tab"
                />
            ));
            const wrapper = container.querySelector(".pane-tab-strip-add-tip") as HTMLElement;
            expect(wrapper).not.toBeNull();
            fireEvent.mouseEnter(wrapper);
            // isOpen flips synchronously on hover (mounts the Portal'd div),
            // but isVisible/opacity is still gated behind the delayMs-
            // scheduled timeout — not visible yet, even at delayMs={0}
            // (still a real setTimeout(fn, 0), not synchronous).
            let tip = document.body.querySelector("[data-pane-overlay]") as HTMLElement;
            expect(tip).not.toBeNull();
            expect(tip.getAttribute("style")).toContain("opacity: 0");
            // Advance far less than the Tooltip's own 300ms default — with
            // delayMs={0} this is enough for the showTimeout to fire; with
            // the default it would not be, which is exactly what this test
            // guards against regressing to.
            vi.advanceTimersByTime(10);
            tip = document.body.querySelector("[data-pane-overlay]") as HTMLElement;
            expect(tip.getAttribute("style")).toContain("opacity: 1");
        } finally {
            vi.useRealTimers();
        }
    });

    // The agent pane's "+ New Agent". Opt-in per pane so the editor and
    // terminal strips keep the bare 28×28px glyph.
    it("renders visible text beside the + when addLabel is given", () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                onAdd={vi.fn()}
                addTitle="New agent"
                addLabel="New Agent"
            />
        ));
        // Named by the label, not the tooltip — the visible text is what a
        // screen reader should announce once there is one.
        const addBtn = screen.getByRole("button", { name: "New Agent" });
        expect(addBtn.textContent).toContain("+");
        expect(addBtn.textContent).toContain("New Agent");
        expect(container.querySelector(".pane-tab-strip-add-labeled")).toBe(addBtn);
    });

    it("stays a bare glyph — no label span, no widening class — without addLabel", () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                onAdd={vi.fn()}
                addTitle="New shell tab"
            />
        ));
        const addBtn = screen.getByRole("button", { name: "New shell tab" });
        expect(addBtn.textContent).toBe("+");
        expect(container.querySelector(".pane-tab-strip-add-label")).toBeNull();
        expect(container.querySelector(".pane-tab-strip-add-labeled")).toBeNull();
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

// SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.2 (Phase 3, same-pane
// reorder): which side of a hovered pill's rect a dragged pill should land
// on. Pure and separately tested since the actual drag gesture (pragmatic-
// dnd, real pointer events) has no unit-test coverage anywhere in this
// codebase — see droppable-tab.tsx/TileLayout.core.tsx, both untested at
// that layer for the same reason (jsdom has no real HTML5 drag/pointer
// pipeline). This function is the one piece of that interaction's logic
// that's pure enough to verify directly.
describe("dropPositionForPointerX", () => {
    const rect = { left: 100, width: 40 } as DOMRect;

    it("returns before when the pointer is left of the pill's midpoint", () => {
        expect(dropPositionForPointerX(rect, 110)).toBe("before");
    });

    it("returns after when the pointer is right of the pill's midpoint", () => {
        expect(dropPositionForPointerX(rect, 130)).toBe("after");
    });

    it("returns after exactly at the midpoint", () => {
        expect(dropPositionForPointerX(rect, 120)).toBe("after");
    });
});

// SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.4: the cross-pane drop zone is
// the target pane's ENTIRE header row, not just its tab strip — a pane with
// one short tab leaves most of the row empty, and aiming at the strip alone
// to move a tab there is fussy. Tested at the element-resolution level for
// the same reason `dropPositionForPointerX` is (no real drag pipeline in
// jsdom), and because getting the element wrong fails SILENTLY: pragmatic-dnd
// never fires a handler for an element the pointer never reaches, which is
// exactly how the cross-pane move in PR #3447 shipped dead in the UI with
// green unit tests (ReAgent P0).
describe("foreignDropRootFor", () => {
    const withHeader = (inner: HTMLElement) => {
        const header = document.createElement("div");
        header.setAttribute("data-role", "block-header");
        header.appendChild(inner);
        return header;
    };

    it("resolves to the enclosing pane header row, not the strip", () => {
        const strip = document.createElement("div");
        const header = withHeader(strip);
        expect(foreignDropRootFor(strip)).toBe(header);
    });

    it("finds the header row through intervening wrappers", () => {
        // PaneHeaderTabStrip hands the strip to BlockFrame_Header as
        // `leadingTabStrip`, which wraps it in its own
        // `.block-frame-default-header-tabstrip` div — so the header is a
        // grandparent, never the direct parent. A parentElement check
        // instead of closest() would miss it.
        const strip = document.createElement("div");
        const wrapper = document.createElement("div");
        wrapper.appendChild(strip);
        const header = withHeader(wrapper);
        expect(foreignDropRootFor(strip)).toBe(header);
    });

    it("falls back to the strip itself when it is not inside a pane header", () => {
        // The editor's file-tab strip and the agent History strip render
        // inside a pane's CONTENT. Neither opts into cross-pane drops
        // today, so this is defensive — but it must never resolve to some
        // unrelated ancestor, which would register the drop target on a
        // region the user has no reason to read as a tab area.
        const strip = document.createElement("div");
        const contentArea = document.createElement("div");
        contentArea.appendChild(strip);
        expect(foreignDropRootFor(strip)).toBe(strip);
    });

    // Repo owner, 2026-09-24: a tab dropped on another pane's BODY does the
    // same as dropping it on the header (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.3).
    const inPane = (inner: HTMLElement) => {
        const pane = document.createElement("div");
        pane.setAttribute("data-role", "pane");
        pane.appendChild(inner);
        return pane;
    };

    it("resolves to the whole pane — header AND body — when the header is inside a pane root", () => {
        const strip = document.createElement("div");
        const header = withHeader(strip);
        const pane = inPane(header);
        pane.appendChild(document.createElement("div")); // the body
        expect(foreignDropRootFor(strip)).toBe(pane);
    });

    it("never widens a strip that lives in a pane's CONTENT to the pane around it", () => {
        // Editor file tabs / the agent History strip: inside a pane, not in
        // its header. Their drop zone must stay the strip itself.
        const strip = document.createElement("div");
        const body = document.createElement("div");
        body.appendChild(strip);
        inPane(withHeader(document.createElement("div"))).appendChild(body);
        expect(foreignDropRootFor(strip)).toBe(strip);
    });

    it("resolves to the NEAREST header when panes are nested in the DOM", () => {
        // Sibling panes aren't nested, but a pane's content can host a
        // surface that has its own header-like chrome; nearest-wins is the
        // only correct reading — a tab must join the pane you're pointing
        // at, never an ancestor pane.
        const strip = document.createElement("div");
        const inner = withHeader(strip);
        const outer = withHeader(inner);
        expect(foreignDropRootFor(strip)).toBe(inner);
        expect(foreignDropRootFor(strip)).not.toBe(outer);
    });
});

describe("foreignDropHighlightFor", () => {
    it("flashes the header row even though the drop zone is the whole pane", () => {
        const strip = document.createElement("div");
        const header = document.createElement("div");
        header.setAttribute("data-role", "block-header");
        header.appendChild(strip);
        const pane = document.createElement("div");
        pane.setAttribute("data-role", "pane");
        pane.appendChild(header);
        expect(foreignDropHighlightFor(strip)).toBe(header);
        expect(foreignDropRootFor(strip)).toBe(pane);
    });

    it("falls back to the strip box outside a header", () => {
        const strip = document.createElement("div");
        expect(foreignDropHighlightFor(strip)).toBe(strip);
    });
});

// SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.6: once the tab strip
// overflows and the user scrolls to its end, there's otherwise no space left
// in a Pane's header to grab for "drag the whole pane" — the strip itself
// has claimed the whole row. `reserveDragHandle` opts a strip into rendering
// a small, non-scrolling-content spacer after the "+" that stays part of the
// ordinary whole-pane drag region (nothing here registers a competing drag
// handler — see PaneHeaderTabStrip.tsx for how the header's own draggable
// treats it). Zero width until the strip is actually overflowing, so it
// doesn't waste space in the common (non-overflowing) case, where the
// header's own natural dead space already serves this purpose.
describe("PaneTabStrip — reserveDragHandle (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.6)", () => {
    let roCb: (() => void) | undefined;
    let scrollWidth = 100;
    let clientWidth = 100;

    beforeEach(() => {
        roCb = undefined;
        scrollWidth = 100;
        clientWidth = 100;
        Object.defineProperty(HTMLElement.prototype, "scrollWidth", {
            configurable: true,
            get() { return scrollWidth; },
        });
        Object.defineProperty(HTMLElement.prototype, "clientWidth", {
            configurable: true,
            get() { return clientWidth; },
        });
        (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
            constructor(cb: () => void) { roCb = cb; }
            observe() {}
            unobserve() {}
            disconnect() {}
        };
    });

    afterEach(() => {
        vi.restoreAllMocks();
    });

    it("omits the drag-handle spacer entirely when reserveDragHandle is not passed", () => {
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                onAdd={vi.fn()}
            />
        ));
        expect(container.querySelector(".pane-tab-strip-drag-handle")).toBeNull();
    });

    it("renders the spacer at zero width when the strip is not overflowing", () => {
        scrollWidth = 100;
        clientWidth = 100;
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                onAdd={vi.fn()}
                reserveDragHandle
            />
        ));
        roCb?.();
        const handle = container.querySelector(".pane-tab-strip-drag-handle");
        expect(handle).not.toBeNull();
        expect(handle?.classList.contains("pane-tab-strip-drag-handle--active")).toBe(false);
    });

    it("activates the spacer once the strip is overflowing", () => {
        scrollWidth = 250;
        clientWidth = 150;
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                onAdd={vi.fn()}
                reserveDragHandle
            />
        ));
        roCb?.();
        const handle = container.querySelector(".pane-tab-strip-drag-handle");
        expect(handle?.classList.contains("pane-tab-strip-drag-handle--active")).toBe(true);
    });

    it("is the true last child of .pane-tab-strip-inner, after the + button", () => {
        scrollWidth = 250;
        clientWidth = 150;
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                onAdd={vi.fn()}
                addTitle="New tab"
                reserveDragHandle
            />
        ));
        roCb?.();
        const inner = container.querySelector(".pane-tab-strip-inner");
        expect(inner?.lastElementChild?.classList.contains("pane-tab-strip-drag-handle")).toBe(true);
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
        // Element-aware, not a flat stub: when `style.width` is explicitly
        // set (mid-FLIP, or an in-flight transition's pinned target),
        // report THAT value — matching a real browser's behavior and
        // exercising the exact "measuring a still-pinned width returns a
        // stale/corrupted value" hazard reagent's review of PR #2768
        // caught. When cleared, reports `currentWidth` (the test's stand-
        // in for "the DOM's true natural size for the current tab count").
        vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
            this: HTMLElement
        ) {
            const explicit = this.style.width;
            const px = explicit ? parseFloat(explicit) : NaN;
            return { width: Number.isNaN(px) ? currentWidth : px } as DOMRect;
        });
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

    it("re-targets to the current tab count's true natural width when a second change arrives before the first transition's cleanup timeout fires (reagent's review of PR #2768)", async () => {
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
        await Promise.resolve(); // mount: records natural width 100, no animation

        const strip = container.querySelector(".pane-tab-strip") as HTMLElement;

        // First change: 1 -> 2 tabs, natural width grows to 180.
        currentWidth = 180;
        setTabs(TABS);
        await Promise.resolve();
        expect(strip.style.width).toBe("180px"); // mid-transition, still pinned to this target

        // Second change arrives WHILE the first transition's cleanup timer
        // is still pending (no timer has fired yet, width is still pinned
        // to "180px"): 2 -> 3 tabs, natural width grows again to 260. The
        // bug this regresses: measuring the still-pinned "180px" as if it
        // were the new tab count's natural width, comparing it against the
        // stale lastMeasuredWidth of 180, finding them equal, and silently
        // doing nothing — leaving the strip stuck at 180px (correct for 2
        // tabs, wrong for the now-current 3) until the ORIGINAL timer fired.
        currentWidth = 260;
        setTabs([TABS[0], TABS[1], { id: "c", label: "gamma" }]);
        await Promise.resolve();

        expect(strip.style.width).toBe("260px");
        expect(strip.style.transition).toBe("width 160ms ease-out");

        // And it must still clear correctly afterward, not get stuck.
        vi.advanceTimersByTime(200);
        expect(strip.style.width).toBe("");
        expect(strip.style.transition).toBe("");
    });
});

// SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md — a Pane header's pill clicks
// on every tone from its own block; strips that don't opt in never do.
describe("PaneTabStrip activity flash", () => {
    let animate: ReturnType<typeof vi.fn>;
    let original: typeof HTMLElement.prototype.animate;
    beforeEach(() => {
        animate = vi.fn((..._args: unknown[]) => ({ cancel: vi.fn() }) as unknown as Animation);
        original = HTMLElement.prototype.animate;
        HTMLElement.prototype.animate = animate as unknown as typeof HTMLElement.prototype.animate;
    });
    afterEach(() => {
        HTMLElement.prototype.animate = original;
        cleanup();
    });

    function renderStrip(flashOnActivity: boolean) {
        return render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                flashOnActivity={flashOnActivity}
            />
        ));
    }

    it("clicks only the matching pill, including a background stack member", () => {
        const { container } = renderStrip(true);
        emitActivityFlash({ blockId: "b" });
        expect(animate).toHaveBeenCalledTimes(1);
        expect(animate.mock.instances[0]).toBe(container.querySelectorAll(".pane-tab")[1]);
    });

    it("ignores other blocks", () => {
        renderStrip(true);
        emitActivityFlash({ blockId: "zzz" });
        expect(animate).not.toHaveBeenCalled();
    });

    it("a strip without flashOnActivity never clicks", () => {
        renderStrip(false);
        emitActivityFlash({ blockId: "b" });
        expect(animate).not.toHaveBeenCalled();
    });
});
