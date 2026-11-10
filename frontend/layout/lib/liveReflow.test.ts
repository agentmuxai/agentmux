// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The live reflow of a pane drag (liveReflow.ts) on a real layout model: which tiles stand in for their future
 * rect, with what transform, and when the reflow lets go. SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md §4, §5, §6.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot, createSignal } from "solid-js";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { glideEntry, landingRectOf, REFLOW_MAX_PANES, REFLOW_WATCHDOG_MS, type ReflowEntry } from "@/layout/lib/liveReflow";
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

beforeEach(() => {
    rows.clear();
    setFlag(true);
});

afterEach(() => {
    dispose?.();
    dispose = undefined;
    vi.useRealTimers();
});

describe("glideEntry", () => {
    it("translates to the future corner, as the layout's own transform does, and crops to a smaller future size, never scaling", () => {
        const from = { top: 0, left: 0, width: 600, height: 800 };
        expect(glideEntry(from, { top: 0, left: 300, width: 600, height: 800 })).toEqual({
            kind: "glide",
            transform: "translate3d(300px,0px, 0)",
            clip: "inset(0px 0px 0px 0px)",
            moved: false,
        });
        expect(glideEntry(from, { top: 0, left: 0, width: 300, height: 400 }, true)).toEqual({
            kind: "glide",
            transform: "translate3d(0px,0px, 0)",
            clip: "inset(0px 300px 400px 0px)",
            moved: true,
        });
        // A larger future rect is not stretched into: the tile shows at its own size until the drop fills it.
        expect(glideEntry(from, { top: 0, left: 0, width: 1200, height: 800 }).clip).toBe("inset(0px 0px 0px 0px)");
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

    it("glides every pane to its future rect, the dragged one into its landing rect, so no slot is left empty", () => {
        const m = modelOf(three());
        // a goes under c: the column takes the whole width, b and c each lose height, and a's old slot closes.
        drag(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(true);
        const got = entries(m, ["a", "b", "c"]);
        expect(got.a).toEqual(glideEntry(m.additionalProps()["a"].rect, landingRectOf(m)!, true));
        expect(got.b?.kind).toBe("glide");
        expect(got.c?.kind).toBe("glide");
        expect([got.b!.moved, got.c!.moved]).toEqual([false, false]);
        // b's future rect is the layout's own rect for b after the same drop: the transform is a translate to its
        // corner, and the crop takes b's current 600 by 400 down to its share of the height (its width grows).
        const future = m.futureLayout()!;
        const bRect = future.rects[future.leafIds.find((id) => future.leafBlockIds[id] === "b")!];
        const bNow = m.additionalProps()["b"].rect;
        expect(got.b).toEqual(glideEntry(bNow, bRect));
        expect(bRect.width).toBeGreaterThan(bNow.width);
        expect(bRect.height).toBeLessThan(bNow.height);
        // The layout's own rects did not move: they stay the drag's hit-test snapshot.
        expect(m.additionalProps()["b"].rect).toEqual(bNow);
    });

    it("keeps a pane at rest whose rect the drop does not change, with the glide's transition still on", () => {
        // a | b | c of equal widths, and a dropped on b's right: b | a | c. b slides left; c keeps the right third.
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c")));
        drag(m, "a", "b", DropDirection.Right);
        const got = entries(m, ["a", "b", "c"]);
        expect(got.a).toMatchObject({ kind: "glide", moved: true });
        expect(got.b?.kind).toBe("glide");
        expect(got.c).toEqual({ kind: "rest", moved: false });
    });

    it("leaves every pane at rest, the dragged one in its own slot, when the drop would change nothing", () => {
        // a | b, and a dropped on b's left: the same layout, which the reducer refuses as a move, so no drop is
        // shown and nothing glides; a's own slot is never emptied.
        const m = modelOf(row("r", 10, leaf("a"), leaf("b")));
        drag(m, "a", "b", DropDirection.Left);
        expect(m.liveReflowActive()).toBe(false);
        expect(entries(m, ["a", "b"])).toEqual({ a: { kind: "rest", moved: false }, b: { kind: "rest", moved: false } });
    });

    it("glides a pane back to its rest when the next candidate leaves it alone, instead of jumping there", () => {
        // a | b | c | d. a on c's right slides b and c left; a on b's right then leaves c where it was laid out:
        // c is at rest, not without an entry, so its transition carries it back from wherever its glide had it.
        vi.useFakeTimers();
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c"), leaf("d")));
        drag(m, "a", "c", DropDirection.Right);
        expect(entries(m, ["b", "c", "d"])).toMatchObject({ b: { kind: "glide" }, c: { kind: "glide" }, d: { kind: "rest" } });
        preview(m, "a", "b", DropDirection.Right);
        vi.advanceTimersByTime(20);
        expect(entries(m, ["b", "c", "d"])).toMatchObject({ b: { kind: "glide" }, c: { kind: "rest" }, d: { kind: "rest" } });
        // With no candidate at all, every pane is at rest and the reflow shows no drop.
        m.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(false);
        const rest = { kind: "rest", moved: false };
        expect(entries(m, ["a", "b", "c", "d"])).toEqual({ a: rest, b: rest, c: rest, d: rest });
    });

    it("never transforms a browser pane, dragged or not: a native window does not follow CSS", () => {
        const m = modelOf(row("r", 10, leaf("browser-x"), col("col", 10, leaf("b"), leaf("c"))));
        drag(m, "b", "browser-x", DropDirection.Left);
        const got = entries(m, ["browser-x", "b", "c"]);
        expect(got["browser-x"]).toBeUndefined();
        expect(got.b).toMatchObject({ kind: "glide", moved: true });
        expect(got.c?.kind).toBe("glide");
        // The browser pane as the dragged one: it is left where it is too.
        const m2 = modelOf(three());
        m2.treeState.rootNode!.children![0].data!.blockId = "browser-a";
        m2.updateTree();
        drag(m2, "a", "c", DropDirection.Bottom);
        expect(m2.reflowFor("a")).toBeUndefined();
        expect(m2.reflowFor("b")?.kind).toBe("glide");
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
        expect(m.reflowFor("b")).toEqual({ kind: "rest", moved: false });
        drag(m, "a", "c", DropDirection.Bottom);
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(true);
        m.activeDrag._set(false);
        expect(m.liveReflowActive()).toBe(false);
        expect(entries(m, ["a", "b", "c"])).toEqual({ a: undefined, b: undefined, c: undefined });
    });

    it("lets go after the watchdog when the drag sends no sample, and a new sample arms it again", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        drag(m, "a", "c", DropDirection.Bottom);
        vi.advanceTimersByTime(REFLOW_WATCHDOG_MS - 1);
        expect(m.liveReflowActive()).toBe(true);
        vi.advanceTimersByTime(1);
        expect(m.liveReflowActive()).toBe(false);
        // Another candidate is a sample: the reflow is back and the watchdog restarts.
        preview(m, "a", "b", DropDirection.Top);
        expect(m.liveReflowActive()).toBe(true);
    });

    it("does not glide a layout of more panes than the cap", () => {
        const many = row("r", 10, ...Array.from({ length: REFLOW_MAX_PANES + 1 }, (_, i) => leaf(`p${i}`)));
        const m = modelOf(many);
        drag(m, "p0", `p${REFLOW_MAX_PANES}`, DropDirection.Right);
        expect(m.liveReflowActive()).toBe(false);
    });
});
