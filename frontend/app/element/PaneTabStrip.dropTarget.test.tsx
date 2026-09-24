// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for PaneTabStrip's cross-pane drop-target WIRING — which element the
 * `dropTargetForElements` registration actually lands on, and what its
 * callbacks do — for SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.4.
 *
 * Deliberately a SECOND test file for one component (the only such pair in
 * the frontend today) rather than more cases in PaneTabStrip.test.tsx: this
 * needs `vi.mock` on the pragmatic-dnd adapter, and `vi.mock` is hoisted to
 * module scope, so folding it in would silently replace real `draggable()`/
 * `dropTargetForElements()` for all ~45 tests in that file. Keeping the mock
 * quarantined here leaves those exercising the real adapter.
 *
 * Why mock at all, when the rest of this interaction is tested through pure
 * functions (`dropPositionForPointerX`, `foreignDropRootFor`): jsdom has no
 * HTML5 drag pipeline, so a real drag can't be simulated — but the *wiring*
 * between the resolved element, the hover feedback and the callback is
 * precisely where this feature's bugs are invisible. ReAgent's P0 on PR
 * #3447 was exactly that shape: a cross-pane move whose logic was unit-test
 * green while the UI path was dead. Capturing the registration lets those
 * handlers be invoked directly.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const dropTargetCalls: any[] = [];

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    draggable: () => () => {},
    dropTargetForElements: (config: any) => {
        dropTargetCalls.push(config);
        return () => {};
    },
}));

import { paneTabItemType, PaneTabStrip } from "./PaneTabStrip";

afterEach(() => cleanup());
beforeEach(() => {
    dropTargetCalls.length = 0;
});

interface T {
    id: string;
    label: string;
}

const TABS: T[] = [
    { id: "a", label: "alpha" },
    { id: "b", label: "beta" },
];

/**
 * Renders a strip the way PaneHeaderTabStrip does — inside a real
 * `[data-role="block-header"]` row, with the strip nested one wrapper deep
 * (BlockFrame_Header's own `.block-frame-default-header-tabstrip`).
 * `onReorder` is deliberately omitted so the per-pill targets don't
 * register: the single remaining call is then unambiguously the §3.4 one.
 */
function renderInHeader(onReceiveForeignTab = vi.fn(), paneKey = "pane-self") {
    const result = render(() => (
        <div data-role="block-header">
            <div class="block-frame-default-header-tabstrip">
                <PaneTabStrip
                    tabs={TABS}
                    activeId="a"
                    getId={(t: T) => t.id}
                    getLabel={(t: T) => t.label}
                    onActivate={vi.fn()}
                    paneKey={paneKey}
                    onReceiveForeignTab={onReceiveForeignTab}
                />
            </div>
        </div>
    ));
    const header = result.container.querySelector<HTMLElement>('[data-role="block-header"]')!;
    const strip = result.container.querySelector<HTMLElement>(".pane-tab-strip")!;
    return { ...result, header, strip, onReceiveForeignTab, config: dropTargetCalls[0] };
}

describe("PaneTabStrip — cross-pane drop target (§3.4)", () => {
    it("registers the drop target on the whole header row, not on the strip", () => {
        const { config, header, strip } = renderInHeader();
        expect(dropTargetCalls).toHaveLength(1);
        expect(config.element).toBe(header);
        expect(config.element).not.toBe(strip);
    });

    it("registers nothing without onReceiveForeignTab — every existing consumer is untouched", () => {
        render(() => (
            <div data-role="block-header">
                <PaneTabStrip
                    tabs={TABS}
                    activeId="a"
                    getId={(t: T) => t.id}
                    getLabel={(t: T) => t.label}
                    onActivate={vi.fn()}
                />
            </div>
        ));
        expect(dropTargetCalls).toHaveLength(0);
    });

    it("registers nothing when paneKey is missing — without it, canDrop could not tell panes apart", () => {
        render(() => (
            <div data-role="block-header">
                <PaneTabStrip
                    tabs={TABS}
                    activeId="a"
                    getId={(t: T) => t.id}
                    getLabel={(t: T) => t.label}
                    onActivate={vi.fn()}
                    onReceiveForeignTab={vi.fn()}
                />
            </div>
        ));
        expect(dropTargetCalls).toHaveLength(0);
    });

    it("accepts a pill from a DIFFERENT pane and rejects this pane's own", () => {
        const { config } = renderInHeader(vi.fn(), "pane-self");
        const drag = (sourceNodeId: string, type: string = paneTabItemType) => ({
            source: { data: { type, sourceNodeId } },
        });
        expect(config.canDrop(drag("pane-other"))).toBe(true);
        // Same-pane drags belong to the per-pill reorder targets; accepting
        // them here would append the tab to the pane it already lives in.
        expect(config.canDrop(drag("pane-self"))).toBe(false);
        // A whole-pane (tile) or Window Tab drag must fall through to the
        // targets that own those gestures.
        expect(config.canDrop(drag("pane-other", "TILE_ITEM"))).toBe(false);
    });

    it("reports the dragged blockId on drop", () => {
        const { config, onReceiveForeignTab } = renderInHeader();
        config.onDrop({ source: { data: { type: paneTabItemType, sourceNodeId: "pane-other", blockId: "blk-7" } } });
        expect(onReceiveForeignTab).toHaveBeenCalledWith("blk-7");
    });

    it("does not fire on a drop carrying no blockId", () => {
        const { config, onReceiveForeignTab } = renderInHeader();
        config.onDrop({ source: { data: { type: paneTabItemType, sourceNodeId: "pane-other" } } });
        expect(onReceiveForeignTab).not.toHaveBeenCalled();
    });

    it("highlights the whole header while hovering, and clears it on leave", async () => {
        const { config, header, strip } = renderInHeader();
        config.onDragEnter();
        await Promise.resolve();
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(true);
        // Not both: the strip's own highlight would draw a second, inner
        // outline inside the one the header already draws.
        expect(strip.classList.contains("pane-tab-strip--foreign-hover")).toBe(false);

        config.onDragLeave();
        await Promise.resolve();
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(false);
    });

    it("clears the highlight on drop, not just on leave", async () => {
        const { config, header } = renderInHeader();
        config.onDragEnter();
        await Promise.resolve();
        config.onDrop({ source: { data: { type: paneTabItemType, sourceNodeId: "pane-other", blockId: "blk-7" } } });
        await Promise.resolve();
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(false);
    });

    it("leaves no highlight on the header when the strip unmounts mid-hover", async () => {
        // The header element belongs to blockframe.tsx, not to the strip, so
        // nothing guarantees the two go away together. A class left behind
        // on a header that outlives its strip would be a permanent accent
        // outline on a pane nobody is dragging onto. (This used to happen for
        // real under the removed `pane:tabstrip = "multi-only"` setting;
        // kept as a guard for any future path that drops the strip alone.)
        const { config, header, unmount } = renderInHeader();
        config.onDragEnter();
        await Promise.resolve();
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(true);
        unmount();
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(false);
    });

    it("falls back to registering on the strip when there is no header row", () => {
        // The editor file-tab strip / agent History strip case: rendered in
        // a pane's CONTENT. Neither opts in today, so this only guarantees
        // the fallback can never resolve to an unrelated ancestor.
        const { container } = render(() => (
            <div class="some-pane-content">
                <PaneTabStrip
                    tabs={TABS}
                    activeId="a"
                    getId={(t: T) => t.id}
                    getLabel={(t: T) => t.label}
                    onActivate={vi.fn()}
                    paneKey="pane-self"
                    onReceiveForeignTab={vi.fn()}
                />
            </div>
        ));
        const strip = container.querySelector(".pane-tab-strip");
        expect(dropTargetCalls).toHaveLength(1);
        expect(dropTargetCalls[0].element).toBe(strip);
    });

    it("highlights the strip itself in that fallback, since it IS the drop zone", async () => {
        const { container } = render(() => (
            <div class="some-pane-content">
                <PaneTabStrip
                    tabs={TABS}
                    activeId="a"
                    getId={(t: T) => t.id}
                    getLabel={(t: T) => t.label}
                    onActivate={vi.fn()}
                    paneKey="pane-self"
                    onReceiveForeignTab={vi.fn()}
                />
            </div>
        ));
        const strip = container.querySelector(".pane-tab-strip")!;
        dropTargetCalls[0].onDragEnter();
        await Promise.resolve();
        expect(strip.classList.contains("pane-tab-strip--foreign-hover")).toBe(true);
    });
});
