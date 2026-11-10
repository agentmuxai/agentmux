// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The live reflow of a pane drag (liveReflow.ts) on a real layout model: which tiles stand in for their future
 * rect, with what transform, and when the reflow lets go. SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md §4, §5, §6.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot, createSignal } from "solid-js";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { glideTransform, REFLOW_MAX_PANES, REFLOW_WATCHDOG_MS, type ReflowEntry } from "@/layout/lib/liveReflow";
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

describe("glideTransform", () => {
    it("translates to the future corner, as the layout's own transform does, and scales to the future size", () => {
        const from = { top: 0, left: 0, width: 600, height: 800 };
        expect(glideTransform(from, { top: 0, left: 300, width: 600, height: 800 })).toBe("translate3d(300px,0px, 0)");
        expect(glideTransform(from, { top: 0, left: 0, width: 300, height: 400 })).toBe(
            "translate3d(0px,0px, 0) scale(0.5000, 0.5000)"
        );
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

    it("hides the dragged pane and glides every other pane to its future rect, and only those that move", () => {
        const m = modelOf(three());
        // a goes under c: the column takes the whole width, b and c each lose height, and a's old slot closes.
        drag(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(true);
        const got = entries(m, ["a", "b", "c"]);
        expect(got.a).toEqual({ kind: "hidden" });
        expect(got.b?.kind).toBe("glide");
        expect(got.c?.kind).toBe("glide");
        // b's future rect is the layout's own rect for b after the same drop: the transform is a translate to its
        // corner and a scale from b's current 600 by 400 to the full width and its share of the height.
        const future = m.futureLayout()!;
        const bRect = future.rects[future.leafIds.find((id) => future.leafBlockIds[id] === "b")!];
        const bNow = m.additionalProps()["b"].rect;
        expect((got.b as { transform: string }).transform).toBe(glideTransform(bNow, bRect));
        // The layout's own rects did not move: they stay the drag's hit-test snapshot.
        expect(m.additionalProps()["b"].rect).toEqual(bNow);
    });

    it("leaves a pane alone whose rect the drop does not change", () => {
        // a | b | c of equal widths, and a dropped on b's right: b | a | c. b slides left; c keeps the right third.
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c")));
        drag(m, "a", "b", DropDirection.Right);
        const got = entries(m, ["a", "b", "c"]);
        expect(got.a).toEqual({ kind: "hidden" });
        expect(got.b?.kind).toBe("glide");
        expect(got.c).toBeUndefined();
    });

    it("never transforms a browser pane, nor hides a dragged one: a native window does not follow CSS", () => {
        const m = modelOf(row("r", 10, leaf("browser-x"), col("col", 10, leaf("b"), leaf("c"))));
        drag(m, "b", "browser-x", DropDirection.Left);
        const got = entries(m, ["browser-x", "b", "c"]);
        expect(got["browser-x"]).toBeUndefined();
        expect(got.b).toEqual({ kind: "hidden" });
        expect(got.c?.kind).toBe("glide");
        // The browser pane as the dragged one: it is not hidden either.
        const m2 = modelOf(three());
        m2.treeState.rootNode!.children![0].data!.blockId = "browser-a";
        m2.updateTree();
        drag(m2, "a", "c", DropDirection.Bottom);
        expect(m2.reflowFor("a")).toBeUndefined();
        expect(m2.reflowFor("b")?.kind).toBe("glide");
    });

    it("lets go when the drag ends, and when the preview is cleared", () => {
        // The pending action is throttled (10 ms): a change right after another lands a tick later.
        vi.useFakeTimers();
        const m = modelOf(three());
        drag(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(true);
        m.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(false);
        drag(m, "a", "c", DropDirection.Bottom);
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(true);
        m.activeDrag._set(false);
        expect(m.liveReflowActive()).toBe(false);
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
