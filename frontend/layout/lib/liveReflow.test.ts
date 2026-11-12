// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The live reflow of a pane drag (liveReflow.ts) on a real layout model: which rect and which laid-out size each
 * tile takes, when a shrinking tile takes its real size, and when the reflow lets go.
 * SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md §4, §5, §6.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot, createSignal } from "solid-js";
import { beginPageCut, PAGE_CUT_MAX_MS, resetPageCutGateForTests } from "@/app/drag/page-cut-gate";
import { beginPaneCapture, resetPaneCaptureGateForTests } from "@/app/drag/pane-capture-gate";
import { setAnimationSettings } from "@/app/motion/motion";
import { DURATION_BASE_MS } from "@/app/motion/tokens";
import { deferWhileSizesHeld, paneSizesHeld, resetPaneSizeHoldForTests } from "@/app/platform/pane-size-hold";
import { layoutWithoutOf } from "@/layout/lib/futureLayout";
import { LayoutModel } from "@/layout/lib/layoutModel";
import {
    envelope,
    landingRectOf,
    REFLOW_MAX_PANES,
    REFLOW_CLIP,
    REFLOW_SETTLE_MS,
    REFLOW_TRANSFORM,
    reflowEdges,
    reflowSettleMs,
    shownPx,
    wholePx,
    type ReflowEntry,
} from "@/layout/lib/liveReflow";
import { showPlacement } from "@/layout/lib/placementPreview";
import {
    commitTearOff,
    leaveOwnPane,
    leaveWindow,
    restOverOwnPane,
    returnToWindow,
    TEAR_OFF_COMMIT_WAIT_MS,
    TEAR_OFF_REST_MS,
    TEAR_OFF_REST_PX,
} from "@/layout/lib/tearOffPreview";
import {
    DropDirection,
    FlexDirection,
    LayoutTreeActionType,
    type LayoutNode,
    type LayoutTreeComputeMoveNodeAction,
    type LayoutTreeDeleteNodeAction,
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
// A block whose id starts with "browser" is a browser pane: its page is a native window, cut away for a drag.
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

/** The layout's own rect of `id`, as a tile shows it. */
const own = (m: LayoutModel, id: string) => shownPx(m.additionalProps()[id].rect);
/** Where the armed drop leaves the pane showing block `id`, unrounded. */
function rawFutureOf(m: LayoutModel, id: string) {
    const future = m.futureLayout()!;
    return future.rects[future.leafIds.find((n) => future.leafBlockIds[n] === id)!];
}
/** The same, as a tile shows it. */
const futureOf = (m: LayoutModel, id: string) => shownPx(rawFutureOf(m, id));
const sizeOf = (r: { width: number; height: number }) => ({ width: r.width, height: r.height });
/** The size a tile is laid out at for a rect: the layout's own rounding. */
const laidOutAt = (r: Dimensions) => sizeOf(wholePx(r));
/** The size the tile of `id` is laid out at in the layout's own rect, and in the armed drop's. */
const ownSize = (m: LayoutModel, id: string) => laidOutAt(m.additionalProps()[id].rect);
const futureSize = (m: LayoutModel, id: string) => laidOutAt(rawFutureOf(m, id));

beforeEach(() => {
    rows.clear();
    setFlag(true);
    resetPaneCaptureGateForTests();
    resetPageCutGateForTests();
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

    it("rounds a shown rect by its edges, so two panes side by side share one whole px in every layout", () => {
        // A third and two thirds of 800: the edge between them is 266.67 in both.
        const [first, second] = [
            shownPx({ left: 0, top: 0, width: 100, height: 800 / 3 }),
            shownPx({ left: 0, top: 800 / 3, width: 100, height: 1600 / 3 }),
        ];
        expect(first.top + first.height).toBe(second.top);
        expect(second.top + second.height).toBe(800);
        // Rounding the size up instead puts the first pane's far edge a px past its neighbour's near one.
        const up = wholePx({ left: 0, top: 0, width: 100, height: 800 / 3 });
        expect(up.top + up.height).toBe(second.top + 1);
    });

    it("writes a shown rect as its four edges, and reads a tile's place and clip from them", () => {
        expect(reflowEdges({ left: 300, top: 20, width: 40, height: 60 })).toEqual({
            "--reflow-l": "300px",
            "--reflow-t": "20px",
            "--reflow-r": "340px",
            "--reflow-b": "80px",
        });
        expect(REFLOW_TRANSFORM).toBe("translate3d(var(--reflow-l), var(--reflow-t), 0)");
        // From the far edges of whatever size the tile is laid out at, so the shown size does not depend on it.
        expect(REFLOW_CLIP).toBe(
            "inset(0px calc(100% - (var(--reflow-r) - var(--reflow-l))) calc(100% - (var(--reflow-b) - var(--reflow-t))) 0px)"
        );
    });

    it("keeps an edge two neighbours share on one target through a change of candidate: it can only move as one line", () => {
        // a | b | c | d. Both drops leave b and c side by side; between them only where they stand changes.
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c"), leaf("d")));
        const shared = () => [reflowEdges(m.reflowFor("b")!.rect)["--reflow-r"], reflowEdges(m.reflowFor("c")!.rect)["--reflow-l"]];
        drag(m, "a", "c", DropDirection.Right);
        expect(shared()[0]).toBe(shared()[1]);
        const first = shared()[0];
        preview(m, "a", "d", DropDirection.Right);
        expect(shared()[0]).toBe(shared()[1]);
        expect(shared()[0]).toBe(first);
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
        setFlag(false);
        const m = modelOf(three());
        drag(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(false);
        expect(entries(m, ["a", "b", "c"])).toEqual({ a: undefined, b: undefined, c: undefined });
    });

    it("is on when the setting is not set", () => {
        setFlag(undefined);
        const m = modelOf(three());
        drag(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(true);
    });

    it("shows nothing before a drag starts, even with a preview pending", () => {
        const m = modelOf(three());
        preview(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(false);
        expect(m.liveReflowExpected()).toBe(false);
    });

    it("is how the drag shows its drops from its start to its end, between two drops too", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        m.activeDrag._set(true);
        // No drop is shown yet, and none is for a moment between two: the reflow is still the drag's look.
        expect(m.liveReflowActive()).toBe(false);
        expect(m.liveReflowExpected()).toBe(true);
        preview(m, "a", "c", DropDirection.Bottom);
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(true);
        expect(m.liveReflowExpected()).toBe(true);
        m.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(false);
        expect(m.liveReflowExpected()).toBe(true);
        m.activeDrag._set(false);
        expect(m.liveReflowExpected()).toBe(false);
        // Not with the setting off.
        setFlag(false);
        m.activeDrag._set(true);
        expect(m.liveReflowExpected()).toBe(false);
    });

    it("gives every pane the rect the drop leaves it, the dragged one its landing rect, so the layout is covered", () => {
        const m = modelOf(three());
        // a goes under c: the column takes the whole width, b and c each lose height, and a's old slot closes.
        drag(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(true);
        const got = entries(m, ["a", "b", "c"]);
        expect(got.a).toMatchObject({ rect: shownPx(landingRectOf(m)!), moved: true });
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
        expect(m.reflowFor("b")!.box).toEqual({ width: to.width, height: ownSize(m, "b").height });
        vi.advanceTimersByTime(REFLOW_SETTLE_MS - 1);
        expect(m.reflowFor("b")!.box.height).toBe(ownSize(m, "b").height);
        // The glide is over: the tile takes the size the drop lays it out at, a px over what it shows at most.
        vi.advanceTimersByTime(1);
        expect(m.reflowFor("b")!.box).toEqual(futureSize(m, "b"));
        expect(m.reflowFor("b")!.rect).toEqual(to);
        expect(m.reflowFor("b")!.box.height - to.height).toBeLessThanOrEqual(1);
    });

    it("takes a drop sampled again and again as one candidate: computed once, and its shrink still settles", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        drag(m, "a", "c", DropDirection.Bottom);
        const future = m.futureLayout();
        const entry = m.reflowFor("b");
        // A pointer held on the drop goes on being sampled, more often than the glide is long.
        for (let waited = 0; waited < REFLOW_SETTLE_MS; waited += 50) {
            vi.advanceTimersByTime(50);
            preview(m, "a", "c", DropDirection.Bottom);
            if (waited + 50 < REFLOW_SETTLE_MS) expect(m.reflowFor("b")).toBe(entry);
        }
        expect(m.futureLayout()).toBe(future);
        expect(m.reflowFor("b")!.box).toEqual(futureSize(m, "b"));
    });

    it("sizes a pane's content to the tile the reflow lays out, and back to the layout's own size after it", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        const gap = m.gapSizePx();
        const inner = m.getNodeModel(m.leafs().find((l) => l.id === "b")!).innerRect!;
        expect(inner()).toEqual({ width: `600px - ${gap}px`, height: `400px - ${gap}px` });
        drag(m, "a", "c", DropDirection.Bottom);
        const to = futureSize(m, "b");
        // Wider at once, so nothing is blank beside it; shorter only when the glide is over.
        expect(inner()).toEqual({ width: `${to.width}px - ${gap}px`, height: `400px - ${gap}px` });
        vi.advanceTimersByTime(REFLOW_SETTLE_MS);
        expect(inner()).toEqual({ width: `${to.width}px - ${gap}px`, height: `${to.height}px - ${gap}px` });
        m.activeDrag._set(false);
        expect(inner()).toEqual({ width: `600px - ${gap}px`, height: `400px - ${gap}px` });
    });

    it("waits out the glide as this window plays it: at a slower animation speed a shrink settles later", () => {
        vi.useFakeTimers();
        setAnimationSettings({ "animation:preset": "slow" });
        try {
            expect(reflowSettleMs()).toBe(DURATION_BASE_MS * 1.5 + (REFLOW_SETTLE_MS - DURATION_BASE_MS));
            const m = modelOf(three());
            drag(m, "a", "c", DropDirection.Bottom);
            // The designed time is over and the slower glide is not: the tile keeps its height under the clip.
            vi.advanceTimersByTime(REFLOW_SETTLE_MS);
            expect(m.reflowFor("b")!.box.height).toBe(ownSize(m, "b").height);
            vi.advanceTimersByTime(reflowSettleMs() - REFLOW_SETTLE_MS);
            expect(m.reflowFor("b")!.box).toEqual(futureSize(m, "b"));
        } finally {
            setAnimationSettings({});
        }
    });

    it("never lays a pane out for a candidate that is replaced inside the glide, if it ends at the size it had", () => {
        vi.useFakeTimers();
        // a over (b | c). a on c's bottom makes b and c taller; clearing it inside the glide sends them back.
        const m = modelOf(col("r", 10, leaf("a"), row("row", 10, leaf("b"), leaf("c"))));
        const before = ownSize(m, "b");
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
        expect(got.c).toEqual({ rect: own(m, "c"), box: ownSize(m, "c"), moved: false, leaving: false });
    });

    it("leaves every pane on its own rect, the dragged one in its own slot, when the drop would change nothing", () => {
        // a | b, and a dropped on b's left: the same layout, which the reducer refuses as a move, so no drop is
        // shown and nothing glides; a's own slot is never emptied.
        const m = modelOf(row("r", 10, leaf("a"), leaf("b")));
        drag(m, "a", "b", DropDirection.Left);
        expect(m.liveReflowActive()).toBe(false);
        expect(entries(m, ["a", "b"])).toEqual({
            a: { rect: own(m, "a"), box: ownSize(m, "a"), moved: false, leaving: false },
            b: { rect: own(m, "b"), box: ownSize(m, "b"), moved: false, leaving: false },
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

    it("moves a browser pane like any other, dragged or not: its page is cut away for the drag, its tile is DOM", () => {
        const m = modelOf(row("r", 10, leaf("browser-x"), col("col", 10, leaf("b"), leaf("c"))));
        drag(m, "b", "browser-x", DropDirection.Left);
        const got = entries(m, ["browser-x", "b", "c"]);
        expect(got["browser-x"]).toMatchObject({ rect: futureOf(m, "browser-x"), moved: false, leaving: false });
        expect(got["browser-x"]!.rect).not.toEqual(own(m, "browser-x"));
        expect(got.b).toMatchObject({ moved: true });
        expect(got.c).toBeDefined();
        // The browser pane as the dragged one: it goes to its landing rect too.
        const m2 = modelOf(three());
        m2.treeState.rootNode!.children![0].data!.blockId = "browser-a";
        m2.updateTree();
        drag(m2, "a", "c", DropDirection.Bottom);
        expect(m2.reflowFor("a")).toMatchObject({ rect: shownPx(landingRectOf(m2)!), moved: true });
        expect(m2.reflowFor("b")).toBeDefined();
    });

    it("shows no drop while a browser page is still on screen at the drag's start, and never waits for one for long", () => {
        vi.useFakeTimers();
        const m = modelOf(row("r", 10, leaf("browser-x"), col("col", 10, leaf("b"), leaf("c"))));
        const cutOne = beginPageCut();
        const cutTwo = beginPageCut();
        drag(m, "b", "browser-x", DropDirection.Left);
        expect(m.liveReflowActive()).toBe(false);
        for (const id of ["browser-x", "b", "c"]) expect(m.reflowFor(id)).toMatchObject({ rect: own(m, id), moved: false });
        // Every page has to be cut, not the first one only; a cut reported twice counts once.
        cutOne();
        cutOne();
        expect(m.liveReflowActive()).toBe(false);
        cutTwo();
        expect(m.liveReflowActive()).toBe(true);
        expect(m.reflowFor("browser-x")!.rect).toEqual(futureOf(m, "browser-x"));
        // A catcher that never reports stops holding the reflow back.
        const m2 = modelOf(three());
        beginPageCut();
        drag(m2, "a", "c", DropDirection.Bottom);
        expect(m2.liveReflowActive()).toBe(false);
        vi.advanceTimersByTime(PAGE_CUT_MAX_MS);
        expect(m2.liveReflowActive()).toBe(true);
    });

    it("does not wait for a page once the drag is over: a tear-off shown through the release has none to cut", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        m.activeDrag._set(true);
        m.activeDrag._set(false);
        beginPageCut();
        commitTearOff(m, "a");
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(true);
    });

    it("moves nothing while the dragged pane's picture is still being taken, with every tile's entry at rest for the first glide to start from", () => {
        const m = modelOf(three());
        const captured = beginPaneCapture();
        drag(m, "a", "c", DropDirection.Bottom);
        expect(m.liveReflowActive()).toBe(false);
        for (const id of ["a", "b", "c"]) {
            expect(m.reflowFor(id)).toEqual({ rect: own(m, id), box: ownSize(m, id), moved: false, leaving: false });
        }
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
        setFlag(false);
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

describe("the landing zone at the drop", () => {
    it("is not placed by a move the drop has just committed: that move names panes of the layout it made", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        drag(m, "a", "c", DropDirection.Bottom);
        vi.advanceTimersByTime(20);
        expect(m.placeholderTransform()).toEqual(expect.objectContaining({ transform: expect.any(String) }));
        // The drop: the tree is committed first, and the pending move is cleared after it.
        m.treeReducer(m.pendingTreeAction.currentValueAtom());
        expect(m.pendingTreeAction.throttledValueAtom()).toBeTruthy();
        expect(m.futureLayout()).toBeUndefined();
        expect(m.placeholderTransform()).toBeUndefined();
    });
});

describe("the tear-off a drag shows over its own pane (tearOffPreview.ts)", () => {
    /** A drag of `id` whose pointer is over that pane itself, at (x, y). */
    function dragOverSelf(m: LayoutModel, id: string, x = 100, y = 100) {
        m.activeDrag._set(true);
        restOverOwnPane(m, id, x, y);
    }

    it("is the layout without the pane: none for a pane that is not in it, or that is its only pane", () => {
        const m = modelOf(three());
        const without = layoutWithoutOf(m, "a")!;
        expect(without).toMatchObject({ movedId: "a", leaves: true });
        expect(without.leafIds.sort()).toEqual(["b", "c"]);
        expect(without.rects.a).toBeUndefined();
        expect(without.rects.b).toMatchObject({ left: 0, width: 1200 });
        // The model's own tree still has the pane.
        expect(m.leafs().map((l) => l.id).sort()).toEqual(["a", "b", "c"]);
        expect(layoutWithoutOf(m, "nope")).toBeUndefined();
        expect(layoutWithoutOf(modelOf(leaf("only")), "only")).toBeUndefined();
    });

    it("closes the other panes over the slot once the pointer rests there, and the dragged pane is not shown", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        dragOverSelf(m, "a");
        vi.advanceTimersByTime(TEAR_OFF_REST_MS - 1);
        expect(m.liveReflowActive()).toBe(false);
        expect(m.reflowFor("b")).toMatchObject({ rect: own(m, "b"), leaving: false });
        vi.advanceTimersByTime(1);
        expect(m.liveReflowActive()).toBe(true);
        const without = m.tearOffLayout()!;
        expect(m.reflowFor("b")).toMatchObject({ rect: shownPx(without.rects.b), moved: false, leaving: false });
        expect(m.reflowFor("c")).toMatchObject({ rect: shownPx(without.rects.c), moved: false, leaving: false });
        expect(m.reflowFor("b")!.rect).toMatchObject({ left: 0, width: 1200 });
        // The pane itself stays laid out where it was, unseen: it is the picture in hand.
        expect(m.reflowFor("a")).toEqual({ rect: own(m, "a"), box: ownSize(m, "a"), moved: false, leaving: true });
        // No drop is armed, so the picture in hand keeps the pane's own size.
        expect(landingRectOf(m)).toBeUndefined();
    });

    it("waits for the pointer to rest: one that keeps moving over its own pane shows nothing", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        dragOverSelf(m, "a", 100, 100);
        for (let step = 1; step <= 10; step++) {
            vi.advanceTimersByTime(TEAR_OFF_REST_MS - 20);
            restOverOwnPane(m, "a", 100 + step * TEAR_OFF_REST_PX, 100);
            expect(m.liveReflowActive()).toBe(false);
        }
        // It stops; a sample that barely moved does not start the wait again.
        vi.advanceTimersByTime(TEAR_OFF_REST_MS - 20);
        restOverOwnPane(m, "a", 101 + 10 * TEAR_OFF_REST_PX, 100);
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(true);
    });

    it("stays while the pointer moves about its own pane, and goes when it is over another pane", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        dragOverSelf(m, "a");
        vi.advanceTimersByTime(TEAR_OFF_REST_MS);
        restOverOwnPane(m, "a", 400, 300);
        expect(m.reflowFor("a")).toMatchObject({ leaving: true });
        showPlacement(m, { targetId: "c", rect: m.additionalProps().c.rect, offset: { x: 900, y: 790 }, draggedId: "a" }, DropDirection.Bottom);
        vi.advanceTimersByTime(20);
        expect(m.tearOffLayout()).toBeUndefined();
        expect(m.reflowFor("a")).toMatchObject({ rect: shownPx(landingRectOf(m)!), moved: true, leaving: false });
    });

    it("stays through the release until the tree lets the pane go, so nothing springs back in between", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        dragOverSelf(m, "a");
        vi.advanceTimersByTime(TEAR_OFF_REST_MS);
        // The release: the source's handler ends the drag, the layout's commits the tear-off.
        m.activeDrag._set(false);
        commitTearOff(m, "a");
        vi.advanceTimersByTime(TEAR_OFF_COMMIT_WAIT_MS - 1);
        expect(m.reflowFor("b")!.rect).toMatchObject({ left: 0, width: 1200 });
        expect(m.reflowFor("a")).toMatchObject({ leaving: true });
        m.treeReducer({ type: LayoutTreeActionType.DeleteNode, nodeId: "a" } as LayoutTreeDeleteNodeAction);
        expect(m.tearOffLayout()).toBeUndefined();
        expect(entries(m, ["a", "b", "c"])).toEqual({ a: undefined, b: undefined, c: undefined });
        // What was shown is what the layout now is.
        expect(own(m, "b")).toMatchObject({ left: 0, width: 1200 });
    });

    it("shows a tear-off released before the pointer rested, from the release on", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        dragOverSelf(m, "a");
        commitTearOff(m, "a");
        m.activeDrag._set(false);
        vi.advanceTimersByTime(20);
        expect(m.reflowFor("b")!.rect).toMatchObject({ left: 0, width: 1200 });
    });

    it("lets the panes back if the tree never lets the pane go", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        dragOverSelf(m, "a");
        m.activeDrag._set(false);
        commitTearOff(m, "a");
        vi.advanceTimersByTime(TEAR_OFF_COMMIT_WAIT_MS);
        expect(m.tearOffLayout()).toBeUndefined();
        // They glide back from where the tear-off left them (settlingPass.ts), then the layout is all there is.
        expect(m.settlingPass()).toBeDefined();
        vi.advanceTimersByTime(reflowSettleMs());
        expect(entries(m, ["a", "b"])).toEqual({ a: undefined, b: undefined });
    });

    it("ends with a drag that is cancelled or dropped elsewhere", async () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        dragOverSelf(m, "a");
        vi.advanceTimersByTime(TEAR_OFF_REST_MS);
        m.activeDrag._set(false);
        await Promise.resolve();
        expect(m.tearOffLayout()).toBeUndefined();
        // The panes glide back from the close-up (settlingPass.ts); nothing is left of it once that is over.
        expect(m.settlingPass()).toBeDefined();
        vi.advanceTimersByTime(reflowSettleMs());
        expect(entries(m, ["a", "b"])).toEqual({ a: undefined, b: undefined });
        // A wait that was still running goes with the drag too.
        dragOverSelf(m, "a");
        m.activeDrag._set(false);
        await Promise.resolve();
        vi.advanceTimersByTime(TEAR_OFF_REST_MS);
        expect(m.tearOffLayout()).toBeUndefined();
    });

    it("shows the tear-off at once when the pointer leaves the window, and keeps it off the pane's overlay", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        m.activeDrag._set(true);
        leaveWindow(m, "a");
        vi.advanceTimersByTime(20);
        expect(m.liveReflowActive()).toBe(true);
        expect(m.reflowFor("b")!.rect).toMatchObject({ left: 0, width: 1200 });
        expect(m.reflowFor("a")).toMatchObject({ leaving: true });
        // The pointer leaving the pane's overlay on its way out is not the pointer coming back.
        leaveOwnPane(m);
        expect(m.tearOffLayout()).toBeDefined();
        // Back in the window: the panes go back.
        returnToWindow(m);
        vi.advanceTimersByTime(20);
        expect(m.tearOffLayout()).toBeUndefined();
        // The drag goes on: every tile is at rest at its own rect, none leaving.
        expect(m.liveReflowActive()).toBe(false);
        expect(m.reflowFor("b")).toMatchObject({ rect: own(m, "b"), leaving: false });
        expect(m.reflowFor("a")).toMatchObject({ rect: own(m, "a"), leaving: false });
    });

    it("stays through a release outside the window until the tree lets the pane go", async () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        m.activeDrag._set(true);
        leaveWindow(m, "a");
        m.activeDrag._set(false);
        await Promise.resolve();
        vi.advanceTimersByTime(TEAR_OFF_COMMIT_WAIT_MS - 1);
        expect(m.reflowFor("b")!.rect).toMatchObject({ left: 0, width: 1200 });
        m.treeReducer({ type: LayoutTreeActionType.DeleteNode, nodeId: "a" } as LayoutTreeDeleteNodeAction);
        expect(m.tearOffLayout()).toBeUndefined();
        expect(own(m, "b")).toMatchObject({ left: 0, width: 1200 });
    });

    it("keeps the tear-off when a placement sample the throttle held back arrives after the pointer left", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        m.activeDrag._set(true);
        leaveWindow(m, "a");
        showPlacement(m, { targetId: "c", rect: m.additionalProps().c.rect, offset: { x: 900, y: 790 }, draggedId: "a" }, DropDirection.Bottom);
        vi.advanceTimersByTime(20);
        expect(m.tearOffLayout()).toBeDefined();
        expect(m.reflowFor("a")).toMatchObject({ leaving: true });
        // Back in the window, a sample is a placement again.
        returnToWindow(m);
        showPlacement(m, { targetId: "c", rect: m.additionalProps().c.rect, offset: { x: 900, y: 790 }, draggedId: "a" }, DropDirection.Bottom);
        vi.advanceTimersByTime(20);
        expect(m.tearOffLayout()).toBeUndefined();
        expect(m.reflowFor("a")).toMatchObject({ moved: true, leaving: false });
    });

    it("takes a tear-off that is shown outside the window off when the drag is cancelled", async () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        m.activeDrag._set(true);
        leaveWindow(m, "a");
        m.activeDrag._set(false);
        await Promise.resolve();
        returnToWindow(m);
        expect(m.tearOffLayout()).toBeUndefined();
    });

    it("ignores an own-pane sample the throttle held back past the exit, and shows nothing outside a drag", () => {
        vi.useFakeTimers();
        const m = modelOf(three());
        leaveWindow(m, "a");
        expect(m.tearOffLayout()).toBeUndefined();
        m.activeDrag._set(true);
        leaveWindow(m, "a");
        restOverOwnPane(m, "a", 300, 300);
        vi.advanceTimersByTime(TEAR_OFF_REST_MS);
        leaveOwnPane(m);
        // Still outside: only the window's dragenter brings the panes back.
        expect(m.tearOffLayout()).toBeDefined();
        expect(m.reflowFor("a")).toMatchObject({ leaving: true });
        // A release now commits the tear-off, not a cancel.
        m.activeDrag._set(false);
        return Promise.resolve().then(() => {
            vi.advanceTimersByTime(TEAR_OFF_COMMIT_WAIT_MS - 1);
            expect(m.tearOffLayout()).toBeDefined();
        });
    });

    it("shows nothing for the only pane of a layout, and nothing with the setting off", () => {
        vi.useFakeTimers();
        const one = modelOf(leaf("only"));
        dragOverSelf(one, "only");
        vi.advanceTimersByTime(TEAR_OFF_REST_MS);
        expect(one.liveReflowActive()).toBe(false);
        setFlag(false);
        const m = modelOf(three());
        dragOverSelf(m, "a");
        vi.advanceTimersByTime(TEAR_OFF_REST_MS);
        expect(m.reflowFor("b")).toBeUndefined();
    });
});

describe("the tear-off's targets keep the panes that touch touching", () => {
    /** Every pair of panes that share an edge in the layout's own rects shares one in the tear-off's. */
    function sharedEdgesHold(m: LayoutModel, ids: string[], gone: string) {
        const rest = ids.filter((id) => id !== gone);
        const edges = (id: string) => {
            const r = m.reflowFor(id)!.rect;
            return { l: r.left, r: r.left + r.width, t: r.top, b: r.top + r.height };
        };
        const before = (id: string) => {
            const r = own(m, id);
            return { l: r.left, r: r.left + r.width, t: r.top, b: r.top + r.height };
        };
        const bad: string[] = [];
        for (const a of rest) {
            for (const b of rest) {
                const [x, y] = [before(a), before(b)];
                const touchesBefore = x.r === y.l && x.t < y.b && y.t < x.b;
                if (!touchesBefore) continue;
                const [p, q] = [edges(a), edges(b)];
                // They may stop being neighbours (a collapsed branch), but never end apart or overlapping.
                if (q.l < p.r && p.t < q.b && q.t < p.b) bad.push(`${a}|${b} overlap ${p.r - q.l}`);
                if (p.r !== q.l && p.t === q.t && p.b === q.b) bad.push(`${a}|${b} apart ${q.l - p.r}`);
            }
        }
        return bad;
    }
    const five = () =>
        col("root", 10, row("top", 6, leaf("a"), leaf("b"), leaf("c"), leaf("d")), row("low", 4, leaf("e"), leaf("f")));

    for (const gone of ["a", "b", "c", "d", "e", "f"]) {
        it(`leaves no gap or overlap between neighbours when ${gone} is torn off a 4 over 2 layout`, () => {
            vi.useFakeTimers();
            const m = modelOf(five());
            m.activeDrag._set(true);
            restOverOwnPane(m, gone, 10, 10);
            vi.advanceTimersByTime(TEAR_OFF_REST_MS);
            expect(sharedEdgesHold(m, ["a", "b", "c", "d", "e", "f"], gone)).toEqual([]);
        });
    }
});
