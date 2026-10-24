// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Several Pane Tabs dragged together (Ctrl/Cmd+click selection): what a pill puts on the
 * drag, where it refuses to be dropped, and what the pane's drop zone does with it.
 * docs/specs/SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §4.3.
 *
 * Its own file for the reason PaneTabStrip.dropTarget.test.tsx gives: the pragmatic-dnd
 * adapter is mocked so the registered callbacks can be called directly.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const dropTargetCalls: any[] = [];
const draggableCalls: any[] = [];
const payloadCalls: unknown[] = [];

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    draggable: (config: any) => {
        draggableCalls.push(config);
        return () => {};
    },
    dropTargetForElements: (config: any) => {
        dropTargetCalls.push(config);
        return () => {};
    },
}));
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({
    setCurrentDragPayload: (p: unknown) => payloadCalls.push(p),
}));

import { endDrag, session } from "@/app/drag/drag-session";
import { paneTabItemType } from "@/app/drag/drag-types";
import { LANDING_BOUNCE_MS, PaneTabStrip } from "./PaneTabStrip";
import { paneTabSelectionApi, resetPaneTabSelection, togglePaneTabSelected } from "./pane-tab-selection";

interface T {
    id: string;
    label: string;
}
const TABS: T[] = [
    { id: "a", label: "alpha" },
    { id: "b", label: "beta" },
];

beforeEach(() => {
    endDrag("cancel");
    dropTargetCalls.length = 0;
    draggableCalls.length = 0;
    payloadCalls.length = 0;
    resetPaneTabSelection();
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    resetPaneTabSelection();
});

const multi = (blockIds: string[], grabbed = blockIds[0], sourceNodeId = "pane-other") => ({
    source: { data: { type: paneTabItemType, sourceNodeId, blockId: grabbed, blockIds } },
});

describe("the pane's drop zone and a multi-tab drag", () => {
    function renderMany(opts: { accepts?: (ids: readonly string[]) => boolean; many?: any } = {}) {
        const onReceiveForeignTabs = opts.many ?? vi.fn((ids: readonly string[]) => [...ids]);
        const result = render(() => (
            <div data-role="pane">
                <div data-role="block-header">
                    <div class="block-frame-default-header-tabstrip">
                        <PaneTabStrip
                            tabs={TABS}
                            activeId="a"
                            getId={(t: T) => t.id}
                            getLabel={(t: T) => t.label}
                            onActivate={vi.fn()}
                            paneKey="pane-self"
                            onReceiveForeignTab={vi.fn()}
                            onReceiveForeignTabs={onReceiveForeignTabs}
                            acceptsForeignTabs={opts.accepts}
                        />
                    </div>
                </div>
            </div>
        ));
        return { ...result, config: dropTargetCalls[0], onReceiveForeignTabs };
    }

    it("flashes at once for a drag that starts inside this pane, which is over the drop zone from the first moment", async () => {
        const { config, container } = renderMany({ accepts: () => true });
        config.onDragStart();
        await Promise.resolve();
        const header = container.querySelector("[data-role=block-header]")!;
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(true);
        config.onDragLeave();
        await Promise.resolve();
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(false);
    });

    it("is accepted when this pane lacks at least one of the tabs, even by the pane the grabbed tab came from", () => {
        const { config } = renderMany({ accepts: (ids) => ids.some((id) => !["a", "b"].includes(id)) });
        expect(config.canDrop(multi(["x", "y"]))).toBe(true);
        expect(config.canDrop(multi(["a", "x"], "a", "pane-self"))).toBe(true);
        // Everything dragged is already here: nothing to take, so no flash.
        expect(config.canDrop(multi(["a", "b"], "a", "pane-self"))).toBe(false);
    });

    it("without acceptsForeignTabs falls back to the single-tab rule: another pane's drag only", () => {
        const { config } = renderMany();
        expect(config.canDrop(multi(["x", "y"]))).toBe(true);
        expect(config.canDrop(multi(["x", "y"], "x", "pane-self"))).toBe(false);
    });

    it("hands every block and the grabbed one to onReceiveForeignTabs, one task after the drop", () => {
        vi.useFakeTimers();
        const { config, onReceiveForeignTabs } = renderMany();
        config.onDrop(multi(["x", "y", "z"], "y"));
        expect(onReceiveForeignTabs).not.toHaveBeenCalled();
        vi.runOnlyPendingTimers();
        expect(onReceiveForeignTabs).toHaveBeenCalledWith(["x", "y", "z"], "y");
    });

    it("never sends a multi-tab drop through the single-tab callback", () => {
        vi.useFakeTimers();
        const single = vi.fn();
        render(() => (
            <div data-role="block-header">
                <PaneTabStrip
                    tabs={TABS}
                    activeId="a"
                    getId={(t: T) => t.id}
                    getLabel={(t: T) => t.label}
                    onActivate={vi.fn()}
                    paneKey="pane-self"
                    onReceiveForeignTab={single}
                    onReceiveForeignTabs={vi.fn(() => [])}
                />
            </div>
        ));
        dropTargetCalls[0].onDrop(multi(["x", "y"]));
        vi.runOnlyPendingTimers();
        expect(single).not.toHaveBeenCalled();
    });

    it("bounces exactly the tabs that moved, in the destination strip", () => {
        vi.useFakeTimers();
        const { config, container } = renderMany({ many: vi.fn(() => ["a", "b"]) });
        config.onDrop(multi(["a", "b", "x"], "a"));
        vi.advanceTimersByTime(0);
        const landed = [...container.querySelectorAll<HTMLElement>(".pane-tab--landing")].map((el) => el.dataset.tabId);
        expect(landed.sort()).toEqual(["a", "b"]);
        vi.advanceTimersByTime(LANDING_BOUNCE_MS);
        expect(container.querySelectorAll(".pane-tab--landing")).toHaveLength(0);
    });

    it("bounces nothing when the move was refused", () => {
        vi.useFakeTimers();
        const { config, container } = renderMany({ many: vi.fn(() => false) });
        config.onDrop(multi(["a", "x"], "a"));
        vi.advanceTimersByTime(0);
        expect(container.querySelectorAll(".pane-tab--landing")).toHaveLength(0);
    });

    it("a payload with a one-entry list is still a single-tab drop", () => {
        vi.useFakeTimers();
        const { config, onReceiveForeignTabs } = renderMany();
        config.onDrop(multi(["x"], "x"));
        vi.runOnlyPendingTimers();
        expect(onReceiveForeignTabs).not.toHaveBeenCalled();
    });
});

describe("what a pill drags", () => {
    function renderSelectable() {
        render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                paneKey="pane-self"
                sourceTabId="tab-1"
                onReorder={vi.fn(() => true)}
                selection={paneTabSelectionApi}
            />
        ));
        // One draggable and one drop target per pill, in tab order.
        return { drag: (i: number) => draggableCalls[i], target: (i: number) => dropTargetCalls[i] };
    }

    it("carries only itself when nothing is selected", () => {
        const { drag } = renderSelectable();
        expect(drag(0).getInitialData()).toEqual({
            kind: "pane-tab",
            blockId: "a",
            type: paneTabItemType,
            sourceNodeId: "pane-self",
        });
    });

    it("carries the whole selection when the grabbed pill is in it", () => {
        togglePaneTabSelected("a");
        togglePaneTabSelected("b");
        const { drag } = renderSelectable();
        const data = drag(0).getInitialData();
        expect(data.blockId).toBe("a");
        expect([...data.blockIds].sort()).toEqual(["a", "b"]);
    });

    it("carries only itself when it is not in the selection", () => {
        togglePaneTabSelected("b");
        togglePaneTabSelected("x");
        const { drag } = renderSelectable();
        expect(drag(0).getInitialData().blockIds).toBeUndefined();
    });

    it("a multi-tab drag puts every selected pill on the drag session and sets up no tear-off", () => {
        togglePaneTabSelected("a");
        togglePaneTabSelected("b");
        const { drag } = renderSelectable();
        drag(0).onDragStart();
        expect(session()?.source?.blockIds?.slice().sort()).toEqual(["a", "b"]);
        expect(payloadCalls, "no tear-off payload for several tabs").toEqual([]);
    });

    it("a single-tab drag still sets the tear-off payload", () => {
        const { drag } = renderSelectable();
        drag(0).onDragStart();
        expect(payloadCalls).toHaveLength(1);
        expect(session()?.source?.blockIds).toBeUndefined();
    });

    it("a pill's reorder target refuses a multi-tab drag, and still takes a single tab of its pane", () => {
        const { target } = renderSelectable();
        const single = { source: { data: { type: paneTabItemType, blockId: "b", sourceNodeId: "pane-self" } } };
        const many = {
            source: {
                data: { type: paneTabItemType, blockId: "b", sourceNodeId: "pane-self", blockIds: ["b", "c"] },
            },
        };
        expect(target(0).canDrop(single)).toBe(true);
        expect(target(0).canDrop(many)).toBe(false);
    });
});
