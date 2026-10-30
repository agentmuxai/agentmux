// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A single pill dragged over a pane's body (`bodyDrop`): the pane's one drop target reads the
 * pointer's zone and gives the header glow where the pill would join and the placement ghost where
 * it would become a pane of its own. The adapter and the zone maths are mocked, as in
 * PaneTabStrip.dropTarget.test.tsx, so the wiring between zone, feedback and drop is what is tested.
 * docs/specs/SPEC_PANE_TAB_DRAG_BODY_PLACEMENT_AND_EDGE_GLOW_2026_10_29.md §3.1.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const dropTargetCalls: any[] = [];
const payloadCalls: unknown[] = [];
const zone: { current: any } = { current: { kind: "none" } };
const ghostCalls: unknown[][] = [];
const clearCalls: number[] = [];

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    draggable: () => () => {},
    dropTargetForElements: (config: any) => {
        dropTargetCalls.push(config);
        return () => {};
    },
}));
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({
    setCurrentDragPayload: (p: unknown) => payloadCalls.push(p),
}));
vi.mock("@/layout/lib/paneBodyDrop", () => ({
    pillZone: () => zone.current,
    showPillGhost: (...args: unknown[]) => ghostCalls.push(args),
    clearPillGhost: () => clearCalls.push(1),
}));

import { PaneTabStrip } from "./PaneTabStrip";
import { paneTabItemType } from "@/app/drag/drag-types";
import { endDrag } from "@/app/drag/drag-session";
import { resetFlashBudget } from "./drop-blink-clock";

afterEach(() => cleanup());
beforeEach(() => {
    endDrag("cancel");
    resetFlashBudget();
    dropTargetCalls.length = 0;
    payloadCalls.length = 0;
    ghostCalls.length = 0;
    clearCalls.length = 0;
    zone.current = { kind: "none" };
    vi.useFakeTimers();
});
afterEach(() => vi.useRealTimers());

interface T {
    id: string;
    label: string;
}

const TABS: T[] = [
    { id: "a", label: "alpha" },
    { id: "b", label: "beta" },
];

function renderPane(opts: { tabs?: T[]; hasBody?: boolean } = {}) {
    const onReceiveForeignTab = vi.fn(() => true);
    const place = vi.fn((_blockId: string, _side: string): string | false => "new-pane");
    const showOverlay = { _set: vi.fn() };
    const model = { showOverlay } as any;
    const result = render(() => (
        <div data-role="pane">
            <div data-role="block-header">
                <div class="block-frame-default-header-tabstrip">
                    <PaneTabStrip
                        tabs={opts.tabs ?? TABS}
                        activeId="a"
                        getId={(t: T) => t.id}
                        getLabel={(t: T) => t.label}
                        onActivate={vi.fn()}
                        paneKey="pane-self"
                        onReceiveForeignTab={onReceiveForeignTab}
                        bodyDrop={{ model, hasBody: () => opts.hasBody ?? true, place }}
                    />
                </div>
            </div>
        </div>
    ));
    const header = result.container.querySelector<HTMLElement>('[data-role="block-header"]')!;
    return { ...result, header, onReceiveForeignTab, place, showOverlay, config: dropTargetCalls[0] };
}

const pill = (extra: Record<string, unknown> = {}) => ({
    source: { data: { type: paneTabItemType, blockId: "dragged", sourceNodeId: "pane-other", ...extra } },
    location: { current: { input: { clientX: 40, clientY: 300 } } },
});

/** Let the 30 ms follow throttle open between two events. */
const later = () => vi.advanceTimersByTime(50);

describe("PaneTabStrip bodyDrop", () => {
    it("a single pill from this very pane is accepted only while another tab stays behind", () => {
        const two = renderPane();
        const own = { source: { data: { type: paneTabItemType, blockId: "a", sourceNodeId: "pane-self" } } };
        expect(two.config.canDrop(own)).toBe(true);
        cleanup();
        dropTargetCalls.length = 0;
        const one = renderPane({ tabs: [TABS[0]] });
        expect(one.config.canDrop(own)).toBe(false);
    });

    it("a pill over the body's edge shows the ghost, brings the overlay on screen, and does not glow the header", () => {
        const { config, header, showOverlay } = renderPane();
        zone.current = { kind: "place", direction: 2, side: "left" };

        config.onDragEnter(pill());

        expect(ghostCalls).toHaveLength(1);
        expect(showOverlay._set).toHaveBeenCalledWith(true);
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(false);
    });

    it("the same direction held does not show the ghost again; a new one does", () => {
        const { config } = renderPane();
        zone.current = { kind: "place", direction: 2, side: "left" };
        config.onDragEnter(pill());
        later();
        config.onDrag(pill());
        expect(ghostCalls).toHaveLength(1);

        zone.current = { kind: "place", direction: 1, side: "right" };
        later();
        config.onDrag(pill());
        expect(ghostCalls).toHaveLength(2);
    });

    it("moving from the edge to the middle takes the ghost down and glows the header", () => {
        const { config, header } = renderPane();
        zone.current = { kind: "place", direction: 2, side: "left" };
        config.onDragEnter(pill());

        zone.current = { kind: "join" };
        later();
        config.onDrag(pill());

        expect(clearCalls).toHaveLength(1);
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(true);
    });

    it("leaving takes the ghost and the overlay down", () => {
        const { config, showOverlay } = renderPane();
        zone.current = { kind: "place", direction: 2, side: "left" };
        config.onDragEnter(pill());

        config.onDragLeave();

        expect(clearCalls).toHaveLength(1);
        expect(showOverlay._set).toHaveBeenLastCalledWith(false);
    });

    it("dropping on a place zone splits the tab out on that side, next task, and does not join", () => {
        const { config, place, onReceiveForeignTab } = renderPane();
        zone.current = { kind: "place", direction: 2, side: "left" };
        config.onDragEnter(pill());

        config.onDrop(pill());
        expect(payloadCalls).toEqual([null]);
        expect(place).not.toHaveBeenCalled();
        vi.runAllTimers();

        expect(place).toHaveBeenCalledWith("dragged", "left");
        expect(onReceiveForeignTab).not.toHaveBeenCalled();
    });

    it("dropping on a join zone joins the pane's tabs, as before", () => {
        const { config, place, onReceiveForeignTab } = renderPane();
        zone.current = { kind: "join" };
        config.onDragEnter(pill());

        config.onDrop(pill());
        vi.runAllTimers();

        expect(onReceiveForeignTab).toHaveBeenCalledWith("dragged");
        expect(place).not.toHaveBeenCalled();
    });

    it("dropping where nothing takes the pill does nothing, and leaves it to the tear-off monitor", () => {
        const { config, place, onReceiveForeignTab } = renderPane();
        zone.current = { kind: "none" };
        config.onDragEnter(pill());

        config.onDrop(pill());
        vi.runAllTimers();

        expect(payloadCalls).toEqual([]);
        expect(place).not.toHaveBeenCalled();
        expect(onReceiveForeignTab).not.toHaveBeenCalled();
    });

    it("several tabs dragged together keep joining anywhere on the pane, with no ghost", () => {
        const { config, header, onReceiveForeignTab } = renderPane();
        zone.current = { kind: "place", direction: 2, side: "left" };
        const many = pill({ blockIds: ["dragged", "other"] });

        config.onDragEnter(many);
        later();
        config.onDrag(many);

        expect(ghostCalls).toHaveLength(0);
        expect(header.classList.contains("pane-header--foreign-hover")).toBe(true);
        config.onDrop(many);
        vi.runAllTimers();
        expect(onReceiveForeignTab).toHaveBeenCalledWith("dragged");
    });
});
