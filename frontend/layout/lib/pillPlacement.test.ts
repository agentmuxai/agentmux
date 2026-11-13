// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A single pane tab placed as a pane of its own (pillPlacement.ts), on a real layout model: the drop is shown
 * by the live reflow as the layout the release makes, the pointer is read against the layout's own rects, and
 * the release commits what is shown.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot, createSignal } from "solid-js";
import { beginDrag, endDrag } from "@/app/drag/drag-session";
import { resetPageCutGateForTests } from "@/app/drag/page-cut-gate";
import { resetPaneCaptureGateForTests } from "@/app/drag/pane-capture-gate";
import { resetPaneSizeHoldForTests } from "@/app/platform/pane-size-hold";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { shownPx } from "@/layout/lib/liveReflow";
import { pillDragIn, pillPlacingIn } from "@/layout/lib/pillDragState";
import { beginPillPlacement, dropPill, followPill, panePillZone, paneReadsPill, registerPillPane, releasePill } from "@/layout/lib/pillPlacement";
import { FlexDirection, type LayoutNode } from "@/layout/lib/types";
import { setPlatform } from "@/util/platformutil";
import type { SignalAtom } from "@/util/util";

const rows = new Map<string, SignalAtom<LayoutState>>();
const h = vi.hoisted(() => ({ payloadCleared: 0 }));
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({
    setCurrentDragPayload: () => void h.payloadCleared++,
    getCurrentDragPayload: () => null,
}));
vi.mock("@/app/store/rpc-util", () => ({
    TabRpcClient: { rpcCall: async () => ({ applied: true, version: Number.MAX_SAFE_INTEGER }) },
}));
vi.mock("@/app/store/global", () => ({
    MOS: {
        makeORef: (_otype: string, oid: string) => oid,
        getMuxObjectAtom: (oid: string) => {
            if (!rows.has(oid)) {
                const [get, set] = createSignal<LayoutState>({ otype: "layout", oid, version: 1, meta: {} } as LayoutState);
                const atom = () => get();
                (atom as any)._set = set;
                rows.set(oid, atom as unknown as SignalAtom<LayoutState>);
            }
            return rows.get(oid);
        },
        getObjectValue: () => undefined,
        setObjectValue: () => {},
    },
    globalStore: { get: () => undefined, set: () => {} },
}));
vi.mock("@/app/store/block-atom-cache", () => ({ getSettingsKeyAtom: () => () => undefined }));
vi.mock("@/app/store/mos", () => ({
    makeORef: (_otype: string, oid: string) => oid,
    getObjectValue: () => ({ meta: { view: "term" } }),
}));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: { requestNodeFocus: () => {}, refocusNode: () => {}, claimFocusOnMount: () => {}, ensureSelectionFocused: () => {} },
}));

const leaf = (id: string, size = 10): LayoutNode => ({ id, size, data: { blockId: id } }) as LayoutNode;
const row = (id: string, size: number, ...children: LayoutNode[]): LayoutNode =>
    ({ id, size, flexDirection: FlexDirection.Row, children }) as LayoutNode;
const col = (id: string, size: number, ...children: LayoutNode[]): LayoutNode =>
    ({ id, size, flexDirection: FlexDirection.Column, children }) as LayoutNode;

let dispose: (() => void) | undefined;

/** a | (b over c) in 1200 by 800: a is 600 wide, b and c are 600 by 400. With `tabs`, pane a holds tabs a and a2. */
function layout(tabs = false): LayoutModel {
    return createRoot((d) => {
        dispose = d;
        const [tab] = createSignal<Tab>({ otype: "tab", oid: "tab-1", version: 1, meta: {}, name: "t", layoutstate: "layout-1", blockids: [] });
        const m = new LayoutModel(tab);
        m.getBoundingRect = () => ({ top: 0, left: 0, width: 1200, height: 800 });
        m.displayContainerRef.current = { getBoundingClientRect: () => ({ x: 0, y: 0, top: 0, left: 0, width: 1200, height: 800 }) } as any;
        m.treeState.rootNode = structuredClone(row("r", 10, leaf("a"), col("col", 10, leaf("b"), leaf("c"))));
        if (tabs) m.treeState.rootNode.children![0].data!.blockStack = ["a", "a2"];
        m.updateTree();
        // srv stores the same tree: a quiet model shows what srv stores (layoutIntents.ts).
        const stored = { otype: "layout", oid: "layout-1", version: 1, meta: {}, rootnode: structuredClone(m.treeState.rootNode) };
        (rows.get("layout-1") as unknown as { _set: (row: unknown) => void })?._set(stored);
        return m;
    });
}

const own = (m: LayoutModel, id: string) => shownPx(m.additionalProps()[id].rect);
const blocksOf = (m: LayoutModel) => m.leafs().map((n) => n.data?.blockId);
/** Near the left edge of pane b, and in its middle. */
const LEFT_OF_B = { x: 640, y: 200 };
const MIDDLE_OF_B = { x: 900, y: 200 };
const RIGHT_OF_B = { x: 1160, y: 200 };
/** Longer than the pending action's throttle holds a clearing back. */
const CLEARED_MS = 100;

beforeEach(() => {
    rows.clear();
    h.payloadCleared = 0;
    setPlatform("win32");
    resetPaneCaptureGateForTests();
    resetPageCutGateForTests();
    resetPaneSizeHoldForTests();
});

afterEach(() => {
    endDrag("dragend");
    dispose?.();
    dispose = undefined;
    vi.useRealTimers();
});

describe("a tab that leaves other tabs behind", () => {
    it("is shown as the split its release makes: the hovered pane in half, the new pane on the pointer's side", () => {
        const m = layout(true);
        expect(beginPillPlacement(m, "a2", LEFT_OF_B)).toBe(true);
        expect(pillPlacingIn(m)).toBe(true);
        expect(m.showOverlay()).toBe(true);
        const future = m.futureLayout()!;
        expect(future.movedId).toBe("pill:a2");
        expect(shownPx(future.rects["pill:a2"])).toEqual({ left: 600, top: 0, width: 300, height: 400 });
        // The panes are the outline: b gives up its left half, and nothing else moves.
        expect(m.liveReflowActive()).toBe(true);
        expect(m.liveReflowExpected()).toBe(true);
        expect(m.reflowFor("b")).toMatchObject({ rect: { left: 900, top: 0, width: 300, height: 400 } });
        expect(m.reflowFor("a")).toMatchObject({ rect: own(m, "a"), moved: false });
        expect(m.reflowFor("c")).toMatchObject({ rect: own(m, "c"), moved: false });
    });

    it("does not take its own pane to the landing zone when it is the pane's first tab", () => {
        const m = layout(true);
        beginPillPlacement(m, "a", LEFT_OF_B);
        expect(m.futureLayout()!.movedId).toBe("pill:a");
        expect(m.reflowFor("a")).toMatchObject({ rect: own(m, "a"), moved: false });
    });

    it("lands where it was shown: the release commits that split, one task later, and nothing stays shown", () => {
        vi.useFakeTimers();
        const m = layout(true);
        beginPillPlacement(m, "a2", LEFT_OF_B);
        const shown = shownPx(m.futureLayout()!.rects["pill:a2"]);
        dropPill(m, LEFT_OF_B);
        // Handled in the window, and still shown until the commit.
        expect(h.payloadCleared).toBe(1);
        expect(m.futureLayout()).toBeDefined();
        vi.advanceTimersByTime(0);
        expect(blocksOf(m).sort()).toEqual(["a", "a2", "b", "c"]);
        const landed = m.leafs().find((n) => n.data?.blockId === "a2")!;
        expect(own(m, landed.id)).toEqual(shown);
        expect(pillPlacingIn(m)).toBe(false);
        expect(m.futureLayout()).toBeUndefined();
        expect(m.showOverlay()).toBe(false);
    });

    it("a quick drag lands on the side under the pointer, not the side it came in by", () => {
        vi.useFakeTimers();
        const now = vi.spyOn(performance, "now");
        const m = layout(true);
        // In by b's left edge, where the first drop is shown at once, and straight on to its right edge.
        now.mockReturnValue(1000);
        beginPillPlacement(m, "a2", LEFT_OF_B);
        expect(pillDragIn(m)!.armed).toMatchObject({ targetId: "b" });
        const cameInBy = pillDragIn(m)!.armed!.direction;
        // Released there 60 ms later, the pointer still travelling: no sample had shown the right side yet.
        now.mockReturnValue(1060);
        expect(dropPill(m, RIGHT_OF_B)).toBe(true);
        expect(pillDragIn(m)!.armed!.direction).not.toBe(cameInBy);
        vi.advanceTimersByTime(CLEARED_MS);
        now.mockRestore();
        // b's right half is the new pane; b keeps its left half.
        const landed = m.leafs().find((n) => n.data?.blockId === "a2")!;
        expect(m.getNodeRectById(landed.id)!.left).toBeGreaterThan(m.getNodeRectById("b")!.left);
        expect(m.getNodeRectById(landed.id)!.top).toBe(m.getNodeRectById("b")!.top);
        expect(pillPlacingIn(m)).toBe(false);
        expect(m.futureLayout()).toBeUndefined();
    });

    it("released where the tab would join a pane, nothing is placed and the release is the pane's", () => {
        vi.useFakeTimers();
        const m = layout(true);
        beginPillPlacement(m, "a2", LEFT_OF_B);
        expect(dropPill(m, MIDDLE_OF_B)).toBe(false);
        expect(pillPlacingIn(m)).toBe(false);
        expect(h.payloadCleared).toBe(0);
        vi.advanceTimersByTime(CLEARED_MS);
        expect(blocksOf(m).sort()).toEqual(["a", "b", "c"]);
    });

    it("is placed through the hovered pane's own strip when it has one, which marks the landing", () => {
        vi.useFakeTimers();
        const m = layout(true);
        const place = vi.fn(() => "b");
        registerPillPane(m, "b", { headerEl: { getBoundingClientRect: () => ({ height: 33 }) } as HTMLElement, hasBody: () => true, place, release: () => {} });
        beginPillPlacement(m, "a2", LEFT_OF_B);
        dropPill(m, LEFT_OF_B);
        vi.advanceTimersByTime(0);
        expect(place).toHaveBeenCalledWith("a2", expect.objectContaining({ kind: "place", side: "left" }));
    });
});

describe("a pane's only tab", () => {
    it("moves the pane itself: its tile goes to the landing rect, as a dragged pane's does", () => {
        const m = layout();
        expect(beginPillPlacement(m, "c", LEFT_OF_B)).toBe(true);
        const future = m.futureLayout()!;
        expect(future.movedId).toBe("c");
        expect(m.reflowFor("c")).toMatchObject({ rect: shownPx(future.rects.c), moved: true });
    });

    it("lands where it was shown", () => {
        vi.useFakeTimers();
        const m = layout();
        beginPillPlacement(m, "c", LEFT_OF_B);
        const shown = shownPx(m.futureLayout()!.rects.c);
        dropPill(m, LEFT_OF_B);
        vi.advanceTimersByTime(0);
        expect(own(m, "c")).toEqual(shown);
        expect(pillPlacingIn(m)).toBe(false);
    });

    it("has no drop inside its own pane: the placing ends there", () => {
        vi.useFakeTimers();
        const m = layout();
        beginPillPlacement(m, "c", LEFT_OF_B);
        followPill(m, { x: 900, y: 600 });
        expect(pillPlacingIn(m)).toBe(false);
        vi.advanceTimersByTime(CLEARED_MS);
        expect(m.futureLayout()).toBeUndefined();
    });
});

describe("giving the drag back to the panes", () => {
    it("ends the placing where the tab would join a pane instead, and for a moment only that pane reads the pointer", () => {
        vi.useFakeTimers();
        const m = layout(true);
        beginPillPlacement(m, "a2", LEFT_OF_B);
        followPill(m, MIDDLE_OF_B);
        expect(pillPlacingIn(m)).toBe(false);
        expect(m.showOverlay()).toBe(false);
        // The panes are still gliding home: the one on screen under the pointer may not be b yet.
        expect(paneReadsPill(m, "b", MIDDLE_OF_B)).toBe(true);
        expect(paneReadsPill(m, "c", MIDDLE_OF_B)).toBe(false);
        vi.advanceTimersByTime(CLEARED_MS);
        expect(m.futureLayout()).toBeUndefined();
    });

    it("reads the header band where the pane stands in the layout, not where the reflow shows it", () => {
        const m = layout(true);
        registerPillPane(m, "b", { headerEl: { getBoundingClientRect: () => ({ height: 33 }) } as HTMLElement, hasBody: () => true, place: () => false, release: () => {} });
        beginPillPlacement(m, "a2", LEFT_OF_B);
        expect(pillPlacingIn(m)).toBe(true);
        // Over b's header as the layout has it, although the reflow has moved b's tile to the right half.
        followPill(m, { x: 640, y: 10 });
        expect(pillPlacingIn(m)).toBe(false);
    });

    it("leaves a pane alone while the layout has the drag", () => {
        const m = layout(true);
        beginPillPlacement(m, "a2", LEFT_OF_B);
        expect(paneReadsPill(m, "b", LEFT_OF_B)).toBe(false);
    });

    it("shows nothing once the drag has ended somewhere else", () => {
        vi.useFakeTimers();
        const m = layout(true);
        beginDrag("pane-tab", { blockId: "a2" });
        beginPillPlacement(m, "a2", LEFT_OF_B);
        endDrag("dragend");
        expect(pillPlacingIn(m)).toBe(false);
        vi.advanceTimersByTime(CLEARED_MS);
        expect(m.futureLayout()).toBeUndefined();
    });
});

describe("a release the layout's catcher gets", () => {
    const header = { getBoundingClientRect: () => ({ height: 33 }) } as HTMLElement;
    const dropAt = (at: { x: number; y: number }) => ({ source: { data: { blockId: "a2" } }, location: { current: { input: { clientX: at.x, clientY: at.y } } } });

    it("is placed by the layout where the tab becomes a pane, and no pane hears of it", () => {
        vi.useFakeTimers();
        const m = layout(true);
        const release = vi.fn();
        registerPillPane(m, "b", { headerEl: header, hasBody: () => true, place: () => "b", release });
        beginPillPlacement(m, "a2", LEFT_OF_B);
        releasePill(m, dropAt(LEFT_OF_B));
        // A second drop event for the same release is not a second drop.
        releasePill(m, dropAt(MIDDLE_OF_B));
        vi.advanceTimersByTime(0);
        expect(release).not.toHaveBeenCalled();
        expect(h.payloadCleared).toBe(1);
    });

    it("goes to the pane that stands there where the tab would join it: the pointer had just left the edge", () => {
        vi.useFakeTimers();
        const m = layout(true);
        const [b, c] = [vi.fn(), vi.fn()];
        registerPillPane(m, "b", { headerEl: header, hasBody: () => true, place: () => "b", release: b });
        registerPillPane(m, "c", { headerEl: header, hasBody: () => true, place: () => "c", release: c });
        beginPillPlacement(m, "a2", LEFT_OF_B);
        const drop = dropAt(MIDDLE_OF_B);
        releasePill(m, drop);
        expect(pillPlacingIn(m)).toBe(false);
        expect(b).toHaveBeenCalledTimes(1);
        expect(b).toHaveBeenCalledWith(drop);
        expect(c).not.toHaveBeenCalled();
    });

    it("goes to that pane too when the placing had already ended and the catcher still got the release", () => {
        vi.useFakeTimers();
        const m = layout(true);
        const release = vi.fn();
        registerPillPane(m, "b", { headerEl: header, hasBody: () => true, place: () => "b", release });
        beginPillPlacement(m, "a2", LEFT_OF_B);
        followPill(m, MIDDLE_OF_B);
        expect(pillPlacingIn(m)).toBe(false);
        releasePill(m, dropAt(MIDDLE_OF_B));
        expect(release).toHaveBeenCalledTimes(1);
    });
});

describe("a pane reading the pointer itself", () => {
    const header = { getBoundingClientRect: () => ({ height: 33 }) } as HTMLElement;
    const pane = { headerEl: header, hasBody: () => true, place: () => "b", release: () => {} };
    /** Over b's header as the layout has it: b stands at 600,0. */
    const HEADER_OF_B = { x: 640, y: 10 };
    /** b's header where the reflow still shows it: in the right half, so the pointer is not over it. */
    const glidingHeader = { left: 900, top: 0, right: 1200, bottom: 33 };
    const input = (m: LayoutModel, nodeId: string, pointer: { x: number; y: number }) => ({
        model: m,
        nodeId,
        blockId: "a2",
        pointer,
        headerRect: glidingHeader,
        ownPane: false,
        hasBody: true,
    });

    it("reads nothing while the layout has the drag", () => {
        const m = layout(true);
        registerPillPane(m, "b", pane);
        beginPillPlacement(m, "a2", LEFT_OF_B);
        expect(panePillZone(input(m, "b", HEADER_OF_B))).toEqual({ kind: "none" });
    });

    it("joins over its header band as the layout has it while the panes glide home, wherever the header is on screen", () => {
        vi.useFakeTimers();
        const m = layout(true);
        registerPillPane(m, "b", pane);
        beginPillPlacement(m, "a2", LEFT_OF_B);
        followPill(m, MIDDLE_OF_B);
        expect(pillPlacingIn(m)).toBe(false);
        // By the header on screen this point is the body's left edge, where the tab would be placed.
        expect(panePillZone(input(m, "b", HEADER_OF_B))).toEqual({ kind: "join" });
        // And only the pane that stands there reads it.
        expect(panePillZone(input(m, "c", HEADER_OF_B))).toEqual({ kind: "none" });
    });

    it("reads its own header once nothing glides", () => {
        const m = layout(true);
        registerPillPane(m, "b", pane);
        expect(panePillZone(input(m, "b", HEADER_OF_B))).toMatchObject({ kind: "place" });
        expect(panePillZone({ ...input(m, "b", HEADER_OF_B), headerRect: { left: 600, top: 0, right: 1200, bottom: 33 } })).toEqual({ kind: "join" });
    });
});

describe("a tab that is not in this layout", () => {
    it("is not taken: the pane shows its own ghost for a tab from another window tab", () => {
        const m = layout();
        expect(beginPillPlacement(m, "elsewhere", LEFT_OF_B)).toBe(false);
        expect(pillDragIn(m)).toBeUndefined();
    });
});
