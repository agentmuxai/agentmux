// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The live reflow of a pane drag (liveReflow.ts) on a real layout model: which rect and which laid-out size each
 * tile takes, when a shrinking tile takes its real size, and when the reflow lets go.
 * SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md §4, §5, §6.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot, createSignal } from "solid-js";
import { beginPaneCapture, resetPaneCaptureGateForTests } from "@/app/drag/pane-capture-gate";
import { deferWhileSizesHeld, paneSizesHeld, resetPaneSizeHoldForTests } from "@/app/platform/pane-size-hold";
import { LayoutModel } from "@/layout/lib/layoutModel";
import {
    clipTo,
    envelope,
    landingRectOf,
    REFLOW_MAX_PANES,
    REFLOW_SETTLE_MS,
    wholePx,
    type ReflowEntry,
} from "@/layout/lib/liveReflow";
import {
    DropDirection,
    FlexDirection,
    LayoutTreeActionType,
    type LayoutNode,
    type LayoutTreeComputeMoveNodeAction,
} from "@/layout/lib/types";
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

const leaf = (id: string, size = 10): LayoutNode => ({ id, size, data: { blockId: id } }) as LayoutNode;
const row = (id: string, size: number, ...children: LayoutNode[]): LayoutNode =>
    ({ id, size, flexDirection: FlexDirection.Row, children }) as LayoutNode;
const col = (id: string, size: number, ...children: LayoutNode[]): LayoutNode =>
    ({ id, size, flexDirection: FlexDirection.Column, children }) as LayoutNode;

/** a | (b over c): 1200 by 800, so a is 600 wide and b, c are 600 by 400. */
const three = () => row("r", 10, leaf("a"), col("col", 10, leaf("b"), leaf("c")));

let dispose: (() => void) | undefined;

function modelOf(tree: LayoutNode): LayoutModel {
    return createRoot((d) => {
        dispose = d;
        const [tab] = createSignal<Tab>({ otype: "tab", oid: "tab-1", version: 1, meta: {}, name: "t", layoutstate: "layout-1", blockids: [] });
        const m = new LayoutModel(tab);
        m.getBoundingRect = () => ({ top: 0, left: 0, width: 1200, height: 800 });
        m.displayContainerRef.current = { getBoundingClientRect: () => ({ top: 0, left: 0, width: 1200, height: 800 }) } as any;
        m.treeState.rootNode = structuredClone(tree);
        m.updateTree();
        return m;
    });
}

/** Start a drag of `dragged` and preview its drop on `target`'s `direction` side. */
function drag(m: LayoutModel, dragged: string, target: string, direction: DropDirection) {
    m.activeDrag._set(true);
    preview(m, dragged, target, direction);
}

/** Preview a drop of `dragged` on `target`'s `direction` side, without starting a drag. */
function preview(m: LayoutModel, dragged: string, target: string, direction: DropDirection) {
    m.treeReducer({
        type: LayoutTreeActionType.ComputeMove,
        nodeId: target,
        nodeToMoveId: dragged,
        direction,
    } as LayoutTreeComputeMoveNodeAction);
}

const entries = (m: LayoutModel, ids: string[]): Record<string, ReflowEntry | undefined> =>
    Object.fromEntries(ids.map((id) => [id, m.reflowFor(id)]));

/** The layout's own rect of `id`, in the whole px a tile is drawn at. */
const own = (m: LayoutModel, id: string) => wholePx(m.additionalProps()[id].rect);
/** Where the armed drop leaves the pane showing block `id`, in whole px. */
function futureOf(m: LayoutModel, id: string) {
    const future = m.futureLayout()!;
    return wholePx(future.rects[future.leafIds.find((n) => future.leafBlockIds[n] === id)!]);
}
const sizeOf = (r: { width: number; height: number }) => ({ width: r.width, height: r.height });

beforeEach(() => {
    rows.clear();
    setFlag(true);
    resetPaneCaptureGateForTests();
    resetPaneSizeHoldForTests();
});

afterEach(() => {
    dispose?.();
    dispose = undefined;
    vi.useRealTimers();
});

describe("the pieces of a tile's reflow style", () => {
    it("rounds a rect as the layout's own transform does: the corner down, the size up", () => {
        expect(wholePx({ left: 10.6, top: 3.2, width: 99.1, height: 50 })).toEqual({ left: 10, top: 3, width: 100, height: 50 });
    });

    it("crops to a size from the far edges, so the shown size does not depend on the size laid out", () => {
        expect(clipTo({ width: 300, height: 200 })).toBe("inset(0px calc(100% - 300px) calc(100% - 200px) 0px)");
    });

    it("lays a tile out at the larger of what it has and what it gets, on each axis", () => {
        expect(envelope({ width: 600, height: 400 }, { width: 900, height: 250 })).toEqual({ width: 900, height: 400 });
        expect(envelope({ width: 600, height: 400 }, { width: 300, height: 400 })).toEqual({ width: 600, height: 400 });
    });
});

describe("landingRectOf", () => {
    it("is the dragged pane's rect in the layout the armed drop leaves, and none without a drop", () => {
        const m = modelOf(three());
        expect(landingRectOf(m)).toBeUndefined();
        drag(m, "a", "c", DropDirection.Bottom);
        // a goes under c, at the column's full width and a third of the height.
        expect(landingRectOf(m)).toMatchObject({ width: 1200 });
        expect(landingRectOf(m)!.height).toBeLessThan(400);
    });
});

describe("the live reflow of a pane drag", () => {
    it("shows nothing with the setting off", () => {
        setFlag(undefined);
        const m = modelOf(three());
        drag(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(false);
        expect(entries(m, ["a", "b", "c"])).toEqual({ a: undefined, b: undefined, c: undefined });
    });

    it("shows nothing before a drag starts, even with a preview pending", () => {
        const m = modelOf(three());
        preview(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(false);
    });

    it("gives every pane the rect the drop leaves it, the dragged one its landing rect, so the layout is covered", () => {
        const m = modelOf(three());
        // a goes under c: the column takes the whole width, b and c each lose height, and a's old slot closes.
        drag(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(true);
        const got = entries(m, ["a", "b", "c"]);
        expect(got.a).toMatchObject({ rect: wholePx(landingRectOf(m)!), moved: true });
        expect(got.b).toMatchObject({ rect: futureOf(m, "b"), moved: false });
        expect(got.c).toMatchObject({ rect: futureOf(m, "c"), moved: false });
        // The three future rects tile the whole layout: their areas add up to it, within the gaps' rounding.
        const area = ["a", "b", "c"].reduce((sum, id) => sum + got[id]!.rect.width * got[id]!.rect.height, 0);
        expect(area).toBeGreaterThan(1200 * 800 * 0.97);
        // The layout's own rects did not move: they stay the drag's hit-test snapshot.
        expect(own(m, "b")).toMatchObject({ width: 600, height: 400 });
    });

    it("lays growth out at once and a shrink only when the glide is over: one layout per pane per candidate", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        drag(m, "a", "c", DropDirection.Bottom);
        // b gets wider (600 to 1200) and shorter (400 to a third of 800).
        const to = futureOf(m, "b");
        expect(to.width).toBeGreaterThan(600);
        expect(to.height).toBeLessThan(400);
        // At once: the new width, to be uncovered by the clip, and still the old height under the closing clip.
        expect(m.reflowFor("b")!.box).toEqual({ width: to.width, height: own(m, "b").height });
        vi.advanceTimersByTime(REFLOW_SETTLE_MS - 1);
        expect(m.reflowFor("b")!.box.height).toBe(own(m, "b").height);
        // The glide is over: the tile takes exactly its rect's size.
        vi.advanceTimersByTime(1);
        expect(m.reflowFor("b")!.box).toEqual(sizeOf(to));
        expect(m.reflowFor("b")!.rect).toEqual(to);
    });

    it("takes a drop sampled again and again as one candidate: computed once, and its shrink still settles", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        drag(m, "a", "c", DropDirection.Bottom);
        const to = futureOf(m, "b");
        const future = m.futureLayout();
        const entry = m.reflowFor("b");
        // A pointer held on the drop goes on being sampled, more often than the glide is long.
        for (let waited = 0; waited < REFLOW_SETTLE_MS; waited += 50) {
            vi.advanceTimersByTime(50);
            preview(m, "a", "c", DropDirection.Bottom);
            if (waited + 50 < REFLOW_SETTLE_MS) expect(m.reflowFor("b")).toBe(entry);
        }
        expect(m.futureLayout()).toBe(future);
        expect(m.reflowFor("b")!.box).toEqual(sizeOf(to));
    });

    it("sizes a pane's content to the tile the reflow lays out, and back to the layout's own size after it", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        const gap = m.gapSizePx();
        const inner = m.getNodeModel(m.leafs().find((l) => l.id === "b")!).innerRect!;
        expect(inner()).toEqual({ width: `600px - ${gap}px`, height: `400px - ${gap}px` });
        drag(m, "a", "c", DropDirection.Bottom);
        const to = futureOf(m, "b");
        // Wider at once, so nothing is blank beside it; shorter only when the glide is over.
        expect(inner()).toEqual({ width: `${to.width}px - ${gap}px`, height: `400px - ${gap}px` });
        vi.advanceTimersByTime(REFLOW_SETTLE_MS);
        expect(inner()).toEqual({ width: `${to.width}px - ${gap}px`, height: `${to.height}px - ${gap}px` });
        m.activeDrag._set(false);
        expect(inner()).toEqual({ width: `600px - ${gap}px`, height: `400px - ${gap}px` });
    });

    it("never lays a pane out for a candidate that is replaced inside the glide, if it ends at the size it had", () => {
        vi.useFakeTimers();
        // a over (b | c). a on c's bottom makes b and c taller; clearing it inside the glide sends them back.
        const m = modelOf(col("r", 10, leaf("a"), row("row", 10, leaf("b"), leaf("c"))));
        const before = sizeOf(own(m, "b"));
        drag(m, "a", "c", DropDirection.Right);
        // b gets narrower (a joins its row) and taller (a's row closes): the height is laid out, the width waits.
        const grown = sizeOf(m.reflowFor("b")!.box);
        expect(grown.width).toBe(before.width);
        expect(grown.height).toBeGreaterThan(before.height);
        vi.advanceTimersByTime(60);
        m.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
        vi.advanceTimersByTime(20);
        // Back to its own rect inside the glide: the width never changed, so b was never laid out narrower.
        expect(m.reflowFor("b")!.rect).toEqual(own(m, "b"));
        expect(m.reflowFor("b")!.box.width).toBe(before.width);
        vi.advanceTimersByTime(REFLOW_SETTLE_MS);
        expect(m.reflowFor("b")!.box).toEqual(before);
    });

    it("keeps a pane the drop leaves alone on its own rect, still an entry so its transition stays on", () => {
        // a | b | c of equal widths, and a dropped on b's right: b | a | c. b slides left; c keeps the right third.
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c")));
        drag(m, "a", "b", DropDirection.Right);
        const got = entries(m, ["a", "b", "c"]);
        expect(got.a).toMatchObject({ moved: true });
        expect(got.b!.rect.left).toBeLessThan(own(m, "b").left);
        expect(got.c).toEqual({ rect: own(m, "c"), box: sizeOf(own(m, "c")), moved: false });
    });

    it("leaves every pane on its own rect, the dragged one in its own slot, when the drop would change nothing", () => {
        // a | b, and a dropped on b's left: the same layout, which the reducer refuses as a move, so no drop is
        // shown and nothing glides; a's own slot is never emptied.
        const m = modelOf(row("r", 10, leaf("a"), leaf("b")));
        drag(m, "a", "b", DropDirection.Left);
        expect(m.liveReflowActive()).toBe(false);
        expect(entries(m, ["a", "b"])).toEqual({
            a: { rect: own(m, "a"), box: sizeOf(own(m, "a")), moved: false },
            b: { rect: own(m, "b"), box: sizeOf(own(m, "b")), moved: false },
        });
    });

    it("sends a pane back to its own rect when the next candidate leaves it alone, and all of them with none armed", () => {
        // a | b | c | d. a on c's right slides b and c left; a on b's right then leaves c where it was laid out.
        vi.useFakeTimers();
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c"), leaf("d")));
        drag(m, "a", "c", DropDirection.Right);
        expect(m.reflowFor("c")!.rect.left).toBeLessThan(own(m, "c").left);
        expect(m.reflowFor("d")!.rect).toEqual(own(m, "d"));
        preview(m, "a", "b", DropDirection.Right);
        vi.advanceTimersByTime(20);
        expect(m.reflowFor("c")!.rect).toEqual(own(m, "c"));
        // With no candidate at all, every pane is on its own rect and the reflow shows no drop.
        m.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(false);
        for (const id of ["a", "b", "c", "d"]) expect(m.reflowFor(id)).toMatchObject({ rect: own(m, id), moved: false });
    });

    it("never moves a browser pane, dragged or not: a native window does not follow CSS", () => {
        const m = modelOf(row("r", 10, leaf("browser-x"), col("col", 10, leaf("b"), leaf("c"))));
        drag(m, "b", "browser-x", DropDirection.Left);
        const got = entries(m, ["browser-x", "b", "c"]);
        expect(got["browser-x"]).toBeUndefined();
        expect(got.b).toMatchObject({ moved: true });
        expect(got.c).toBeDefined();
        // The browser pane as the dragged one: it is left where it is too.
        const m2 = modelOf(three());
        m2.treeState.rootNode!.children![0].data!.blockId = "browser-a";
        m2.updateTree();
        drag(m2, "a", "c", DropDirection.Bottom);
        expect(m2.reflowFor("a")).toBeUndefined();
        expect(m2.reflowFor("b")).toBeDefined();
    });

    it("moves nothing while the dragged pane's picture is still being taken", () => {
        const m = modelOf(three());
        const captured = beginPaneCapture();
        drag(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(false);
        expect(m.reflowFor("b")).toBeUndefined();
        captured();
        expect(m.liveReflowActive()).toBe(true);
        expect(m.reflowFor("b")!.rect).toEqual(futureOf(m, "b"));
    });

    it("holds what terminals tell their programs about size for the drag's length, and lets them fit once at its end", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        const fit = vi.fn();
        const onResize = deferWhileSizesHeld(fit);
        expect(paneSizesHeld()).toBe(false);
        drag(m, "a", "c", DropDirection.Bottom);
        expect(paneSizesHeld()).toBe(true);
        // The reflow resizes the terminal's tile: its observer fires, and nothing is refitted or sent.
        onResize();
        onResize();
        expect(fit).not.toHaveBeenCalled();
        // Another candidate, then none, then a third: the hold is never let go between them.
        preview(m, "a", "b", DropDirection.Top);
        vi.advanceTimersByTime(20);
        onResize();
        m.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
        vi.advanceTimersByTime(20);
        preview(m, "a", "c", DropDirection.Bottom);
        vi.advanceTimersByTime(REFLOW_SETTLE_MS);
        onResize();
        expect(paneSizesHeld()).toBe(true);
        expect(fit).not.toHaveBeenCalled();
        m.activeDrag._set(false);
        expect(paneSizesHeld()).toBe(false);
        expect(fit).toHaveBeenCalledTimes(1);
    });

    it("holds no sizes with the setting off: nothing is resized by a drag then", () => {
        setFlag(undefined);
        const m = modelOf(three());
        drag(m, "a", "c", DropDirection.Bottom);
        expect(paneSizesHeld()).toBe(false);
    });

    it("shows no drop while the preview is cleared, and lets every pane go when the drag ends", () => {
        // The pending action is throttled (10 ms): a change right after another lands a tick later.
        vi.useFakeTimers();
        const m = modelOf(three());
        drag(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(true);
        m.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(false);
        expect(m.reflowFor("b")).toMatchObject({ rect: own(m, "b"), moved: false });
        drag(m, "a", "c", DropDirection.Bottom);
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(true);
        m.activeDrag._set(false);
        expect(m.liveReflowActive()).toBe(false);
        expect(entries(m, ["a", "b", "c"])).toEqual({ a: undefined, b: undefined, c: undefined });
    });

    it("holds a drop for as long as the pointer does: no timer lets go of a drag that is still on", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        drag(m, "a", "c", DropDirection.Bottom);
        vi.advanceTimersByTime(60_000);
        expect(m.liveReflowActive()).toBe(true);
        expect(m.reflowFor("b")!.rect).toEqual(futureOf(m, "b"));
    });

    it("lets every pane back to its own rect when the layout changes under the drag: the armed move is for a layout that is gone", () => {
        vi.useFakeTimers();
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c"), leaf("d")));
        drag(m, "a", "c", DropDirection.Right);
        expect(m.liveReflowActive()).toBe(true);
        // srv closes d under the drag.
        m.treeState.rootNode!.children!.pop();
        m.updateTree();
        expect(m.futureLayout()).toBeUndefined();
        expect(m.liveReflowActive()).toBe(false);
        expect(m.reflowFor("b")).toMatchObject({ rect: own(m, "b"), moved: false });
        // The drag's next sample arms a candidate on the new layout.
        preview(m, "a", "c", DropDirection.Right);
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(true);
    });

    it("does not reflow a layout of more panes than the cap", () => {
        const many = row("r", 10, ...Array.from({ length: REFLOW_MAX_PANES + 1 }, (_, i) => leaf(`p${i}`)));
        const m = modelOf(many);
        drag(m, "p0", `p${REFLOW_MAX_PANES}`, DropDirection.Right);
        expect(m.liveReflowActive()).toBe(false);
    });
});
