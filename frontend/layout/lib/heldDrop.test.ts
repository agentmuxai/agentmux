// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A drop that srv commits (heldDrop.ts) on a real layout model: what the release showed stays shown until the
 * layout holds the pane, the pane's arrival moves nothing a second time, and a pane that does not come lets
 * the drop go.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot, createSignal } from "solid-js";
import { resetPaneCaptureGateForTests } from "@/app/drag/pane-capture-gate";
import { setAnimationSettings } from "@/app/motion/motion";
import { resetPaneSizeHoldForTests } from "@/app/platform/pane-size-hold";
import { dropHeldIn, HELD_DROP_MAX_MS, holdDrop, releaseHeldDrop, resetHeldDropForTests } from "@/layout/lib/heldDrop";
import { reconcileWithStoredLayout } from "@/layout/lib/layoutIntents";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { shownPx } from "@/layout/lib/liveReflow";
import { pillStandIn } from "@/layout/lib/paneBodyDrop";
import { clearPlacement, resetDropIntent, showPlacement } from "@/layout/lib/placementPreview";
import { DropDirection, FlexDirection, type LayoutNode } from "@/layout/lib/types";
import type { SignalAtom } from "@/util/util";

const rows = new Map<string, SignalAtom<LayoutState>>();
const [flag, setFlag] = createSignal<boolean | undefined>(true);
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
vi.mock("@/app/store/block-atom-cache", () => ({
    getSettingsKeyAtom: (key: string) => (key === "layout:livereflow" ? flag : () => undefined),
}));
// A block whose id starts with "browser" is a browser pane: a native window a transform cannot move.
vi.mock("@/app/store/mos", () => ({
    makeORef: (_otype: string, oid: string) => oid,
    getObjectValue: (oid: string) => (oid.startsWith("browser") ? { meta: { view: "browser" } } : { meta: { view: "term" } }),
}));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: { requestNodeFocus: () => {}, refocusNode: () => {}, claimFocusOnMount: () => {}, ensureSelectionFocused: () => {} },
}));

// A pane carries the axis it would split along, the other one than its row's (`addChildAt`, layoutNode.ts).
const leaf = (id: string): LayoutNode => ({ id, size: 10, flexDirection: FlexDirection.Column, data: { blockId: id } }) as LayoutNode;

const FRAME_MS = 16;
/** Longer than the pending action's throttle: what was dispatched is what the layout shows. */
const SHOWN_MS = 20;

let dispose: (() => void) | undefined;
const container = { top: 0, left: 0, width: 1200, height: 800 };

/** What srv stores for the tab's layout. */
function store(rootnode: LayoutNode, version: number): void {
    (rows.get("layout-1") as unknown as { _set: (row: unknown) => void })._set({ otype: "layout", oid: "layout-1", version, meta: {}, rootnode });
}

/** Panes a and b side by side, 600 wide each. */
function twoPanes(): LayoutModel {
    return createRoot((d) => {
        dispose = d;
        const [tab] = createSignal<Tab>({ otype: "tab", oid: "tab-1", version: 1, meta: {}, name: "t", layoutstate: "layout-1", blockids: [] });
        const m = new LayoutModel(tab);
        m.getBoundingRect = () => container;
        m.displayContainerRef.current = { getBoundingClientRect: () => container } as any;
        m.treeState.rootNode = { id: "r", size: 10, flexDirection: FlexDirection.Row, children: [leaf("a"), leaf("b")] } as LayoutNode;
        m.updateTree();
        // srv stores the same tree: a quiet model shows what srv stores (layoutIntents.ts).
        store(structuredClone(m.treeState.rootNode), 1);
        return m;
    });
}

/** The drop of a pane from another window tab (block `x`) on the right half of pane a, as its drag shows it. */
function showDrop(m: LayoutModel): void {
    const stand = pillStandIn("x");
    // The spring switch turns the target tab's drag on (app/tab/droppable-tab.tsx).
    m.activeDrag._set(true);
    showPlacement(
        m,
        { targetId: "a", rect: m.getNodeRectById("a"), offset: { x: 500, y: 400 }, draggedId: stand.id, foreignNode: stand, pill: true },
        DropDirection.Right
    );
    vi.advanceTimersByTime(SHOWN_MS);
}

/** The layout srv sends for that drop: pane a split in half, the new pane (block `x`) on its right. */
function paneArrives(m: LayoutModel): void {
    const half = (node: LayoutNode): LayoutNode => ({ ...node, size: 5 });
    const arrived = { id: "n", size: 5, flexDirection: FlexDirection.Column, data: { blockId: "x" } } as LayoutNode;
    store({ id: "r", size: 10, flexDirection: FlexDirection.Row, children: [half(leaf("a")), arrived, leaf("b")] } as LayoutNode, 2);
    reconcileWithStoredLayout(m);
}

/** The release: the move holds the drop (crossTabDrag.ts `moveBlocksToTab`), then the drag is over. */
function release(m: LayoutModel): boolean {
    const held = holdDrop(m, "x");
    clearPlacement(m);
    m.activeDrag._set(false);
    resetDropIntent(m);
    return held;
}

const nodeOf = (m: LayoutModel, blockId: string) => m.leafs().find((leaf) => leaf.data?.blockId === blockId)!.id;
/** Where the pane of `blockId` is drawn: by the reflow while it has the pane, else at its own rect. */
const shown = (m: LayoutModel, blockId: string) => {
    const id = nodeOf(m, blockId);
    return m.reflowFor(id)?.rect ?? shownPx(m.additionalProps()[id].rect);
};

beforeEach(() => {
    rows.clear();
    setFlag(true);
    resetPaneCaptureGateForTests();
    resetPaneSizeHoldForTests();
    resetHeldDropForTests();
    vi.useFakeTimers();
    vi.stubGlobal("requestAnimationFrame", (fn: () => void) => setTimeout(fn, FRAME_MS));
    vi.stubGlobal("cancelAnimationFrame", (id: ReturnType<typeof setTimeout>) => clearTimeout(id));
});

afterEach(() => {
    dispose?.();
    dispose = undefined;
    resetHeldDropForTests();
    setAnimationSettings({});
    vi.unstubAllGlobals();
    vi.useRealTimers();
});

describe("a drop that srv commits", () => {
    it("stays shown when the drag ends and its handlers clear the placement, and the panes stay where it put them", () => {
        const m = twoPanes();
        showDrop(m);
        expect(shown(m, "a")).toMatchObject({ left: 0, width: 300 });

        expect(release(m)).toBe(true);
        expect(dropHeldIn(m)).toBe(true);
        // Whatever else the ended drag runs.
        clearPlacement(m, true);
        clearPlacement(m);
        vi.advanceTimersByTime(SHOWN_MS);

        expect(m.futureLayout()).toBeDefined();
        expect(m.liveReflowActive()).toBe(true);
        expect(shown(m, "a")).toMatchObject({ left: 0, width: 300 });
    });

    it("is committed by the layout that brings the pane: nothing glides a second time", () => {
        const m = twoPanes();
        showDrop(m);
        expect(release(m)).toBe(true);

        paneArrives(m);
        expect(dropHeldIn(m)).toBe(false);
        expect(m.settlingPass()).toBeUndefined();
        vi.advanceTimersByTime(SHOWN_MS);
        expect(m.futureLayout()).toBeUndefined();
        expect(m.pendingTreeAction.currentValueAtom()).toBeUndefined();
        // Each pane is at its own rect, the one the drop showed.
        expect(m.liveReflowActive()).toBe(false);
        expect(m.reflowFor(nodeOf(m, "a"))).toBeUndefined();
        expect(shown(m, "a")).toMatchObject({ left: 0, width: 300 });
        expect(shown(m, "x")).toMatchObject({ left: 300, width: 300 });
    });

    it("glides the panes for the same pane when no drop was held for it", () => {
        const m = twoPanes();
        paneArrives(m);
        expect(m.settlingPass()).toBeDefined();
    });

    it("goes when the pane has not come in time", () => {
        const m = twoPanes();
        showDrop(m);
        expect(release(m)).toBe(true);
        vi.advanceTimersByTime(HELD_DROP_MAX_MS);
        expect(dropHeldIn(m)).toBe(false);
        expect(m.pendingTreeAction.currentValueAtom()).toBeUndefined();
    });

    it("goes when srv refused the move", () => {
        const m = twoPanes();
        showDrop(m);
        expect(release(m)).toBe(true);
        releaseHeldDrop(m);
        expect(dropHeldIn(m)).toBe(false);
        expect(m.pendingTreeAction.currentValueAtom()).toBeUndefined();
    });

    it("holds nothing where no drop is shown", () => {
        const m = twoPanes();
        expect(holdDrop(m, "x")).toBe(false);
        expect(dropHeldIn(m)).toBe(false);
    });

    it("gives way to the drop of the next drag", () => {
        const m = twoPanes();
        showDrop(m);
        expect(release(m)).toBe(true);
        showDrop(m);
        expect(dropHeldIn(m)).toBe(false);
        expect(m.futureLayout()).toBeDefined();
        // That drag's own handlers take its drop down again.
        clearPlacement(m);
        expect(m.pendingTreeAction.currentValueAtom()).toBeUndefined();
    });
});
