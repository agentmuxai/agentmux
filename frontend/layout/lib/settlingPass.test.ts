// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The settling pass (settlingPass.ts) on a real layout model: which panes glide after one closes, opens, is
 * minimized or restored, or two are swapped, from where, what stands in for a closed pane and which panes fade
 * in, when the pass ends, and every case in which the change must not glide.
 * SPEC_PANE_REFLOW_GLIDE_2026_11_01.md §6, §7.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot, createSignal } from "solid-js";
import { beginPaneCapture, resetPaneCaptureGateForTests } from "@/app/drag/pane-capture-gate";
import { setAnimationSettings } from "@/app/motion/motion";
import { holdRevealGate, scheduleRevealLift } from "@/app/store/tab-reveal";
import { deferWhileSizesHeld, paneSizesHeld, resetPaneSizeHoldForTests } from "@/app/platform/pane-size-hold";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { REFLOW_MAX_PANES, reflowSettleMs, shownPx } from "@/layout/lib/liveReflow";
import { planSettlingPass } from "@/layout/lib/settlingPass";
import { commitTearOff, leaveWindow, returnToWindow } from "@/layout/lib/tearOffPreview";
import {
    FlexDirection,
    LayoutTreeActionType,
    type LayoutNode,
    type LayoutNodeAdditionalProps,
    type LayoutTreeDeleteNodeAction,
    type LayoutTreeInsertNodeAction,
    type LayoutTreeMinimizeNodeAction,
    type LayoutTreeResizeNodeAction,
    type LayoutTreeSplitHorizontalAction,
    type LayoutTreeSwapNodeAction,
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

/** One frame, as the fake clock counts it. */
const FRAME_MS = 16;
/** A pass stands at its start until the frame it opened on has been drawn: two frames. */
const SEED_MS = 2 * FRAME_MS;

let dispose: (() => void) | undefined;
let container = { top: 0, left: 0, width: 1200, height: 800 };

function modelOf(tree: LayoutNode): LayoutModel {
    return createRoot((d) => {
        dispose = d;
        const [tab] = createSignal<Tab>({ otype: "tab", oid: "tab-1", version: 1, meta: {}, name: "t", layoutstate: "layout-1", blockids: [] });
        const m = new LayoutModel(tab);
        m.getBoundingRect = () => container;
        m.displayContainerRef.current = { getBoundingClientRect: () => container } as any;
        m.treeState.rootNode = structuredClone(tree);
        m.updateTree();
        return m;
    });
}

const close = (m: LayoutModel, nodeId: string) =>
    m.treeReducer({ type: LayoutTreeActionType.DeleteNode, nodeId } as LayoutTreeDeleteNodeAction);
/** The layout's own rect of `id`, as a tile shows it. */
const own = (m: LayoutModel, id: string) => shownPx(m.additionalProps()[id].rect);
const ghostIds = (m: LayoutModel) => (m.settlingPass()?.ghosts ?? []).map((ghost) => ghost.id);
const open = (m: LayoutModel, id: string) =>
    m.treeReducer({ type: LayoutTreeActionType.InsertNode, node: leaf(id), magnified: false, focused: false } as LayoutTreeInsertNodeAction);
const minimize = (m: LayoutModel, nodeId: string) =>
    m.treeReducer({ type: LayoutTreeActionType.Minimize, nodeId } as LayoutTreeMinimizeNodeAction);
const restore = (m: LayoutModel, nodeId: string) =>
    m.treeReducer({ type: LayoutTreeActionType.Restore, nodeId } as LayoutTreeMinimizeNodeAction);
const swap = (m: LayoutModel, node1Id: string, node2Id: string) =>
    m.treeReducer({ type: LayoutTreeActionType.Swap, node1Id, node2Id } as LayoutTreeSwapNodeAction);
/** The glide's seed frame and its length: the pass is over after this. */
const settle = () => vi.advanceTimersByTime(SEED_MS + reflowSettleMs() + 1);

beforeEach(() => {
    rows.clear();
    setFlag(true);
    container = { top: 0, left: 0, width: 1200, height: 800 };
    resetPaneCaptureGateForTests();
    resetPaneSizeHoldForTests();
    vi.useFakeTimers();
    // The test environment has no frames: one frame is a timer here, so the frame a pass opens on can be seen.
    vi.stubGlobal("requestAnimationFrame", (fn: () => void) => setTimeout(fn, FRAME_MS));
    vi.stubGlobal("cancelAnimationFrame", (id: ReturnType<typeof setTimeout>) => clearTimeout(id));
});

afterEach(() => {
    dispose?.();
    dispose = undefined;
    setAnimationSettings({});
    vi.unstubAllGlobals();
    vi.useRealTimers();
});

describe("a pane closes", () => {
    it("starts the pane that takes its place at the rect it showed, laid out at its new size", () => {
        const m = modelOf(three());
        const before = own(m, "b");
        close(m, "c");
        // The layout is final at once: b is the whole right half.
        expect(own(m, "b")).toEqual({ left: 600, top: 0, width: 600, height: 800 });
        // The tile still stands where it was, already as large as it gets: the clip uncovers the rest.
        expect(m.reflowFor("b")).toMatchObject({ rect: before, box: { width: 600, height: 800 }, moved: false, leaving: false });
        // A pane that did not move does not take part.
        expect(m.reflowFor("a")).toBeUndefined();
        expect(m.liveReflowActive()).toBe(false);
    });

    it("sends it to its own rect once the frame it opened on has been drawn, and lets go when the glide is over", () => {
        const m = modelOf(three());
        const before = own(m, "b");
        close(m, "c");
        // The frame that lays the panes out at their new sizes: the tiles are drawn where they stood.
        vi.advanceTimersByTime(FRAME_MS);
        expect(m.settlingPass()!.gliding).toBe(false);
        expect(m.reflowFor("b")!.rect).toEqual(before);
        vi.advanceTimersByTime(FRAME_MS);
        expect(m.settlingPass()!.gliding).toBe(true);
        expect(m.reflowFor("b")).toMatchObject({ rect: own(m, "b"), box: { width: 600, height: 800 } });
        vi.advanceTimersByTime(reflowSettleMs() - 1);
        expect(m.reflowFor("b")).toBeDefined();
        vi.advanceTimersByTime(1);
        expect(m.reflowFor("b")).toBeUndefined();
        expect(m.settlingPass()).toBeUndefined();
    });

    it("moves every pane that changed place", () => {
        const m = modelOf(three());
        const before = { b: own(m, "b"), c: own(m, "c") };
        close(m, "a");
        expect(m.reflowFor("b")!.rect).toEqual(before.b);
        expect(m.reflowFor("c")!.rect).toEqual(before.c);
        vi.advanceTimersByTime(SEED_MS);
        expect(m.reflowFor("b")!.rect).toEqual({ left: 0, top: 0, width: 1200, height: 400 });
        expect(m.reflowFor("c")!.rect).toEqual({ left: 0, top: 400, width: 1200, height: 400 });
    });

    it("leaves a ghost of the closed pane at its last rect for the length of the pass", () => {
        const m = modelOf(three());
        const rect = m.additionalProps()["c"].rect;
        close(m, "c");
        expect(m.settlingPass()!.ghosts).toEqual([{ id: "c", rect }]);
        vi.advanceTimersByTime(SEED_MS + reflowSettleMs());
        expect(ghostIds(m)).toEqual([]);
    });

    it("leaves a browser pane out: it takes its new rect at once", () => {
        const m = modelOf(row("r", 10, leaf("a"), col("col", 10, leaf("browser-1"), leaf("c"))));
        close(m, "c");
        expect(m.reflowFor("browser-1")).toBeUndefined();
        expect(ghostIds(m)).toEqual(["c"]);
    });

    it("holds the terminals' size for the pass, so each fits once, at its end", () => {
        const m = modelOf(three());
        const fit = vi.fn();
        const onResize = deferWhileSizesHeld(fit);
        close(m, "c");
        expect(paneSizesHeld()).toBe(true);
        // The tile's new size reaches the terminal's observer at the first frame, and again when the pass ends.
        onResize();
        vi.advanceTimersByTime(SEED_MS + reflowSettleMs() - 1);
        onResize();
        expect(fit).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(paneSizesHeld()).toBe(false);
        expect(fit).toHaveBeenCalledTimes(1);
    });
});

describe("a second change during a glide", () => {
    it("retargets the panes already gliding from where they are drawn, and keeps both ghosts", () => {
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c"), leaf("d")));
        const before = own(m, "d");
        close(m, "a");
        vi.advanceTimersByTime(SEED_MS + 60);
        close(m, "b");
        // c was gliding: it has no start of its own, so it goes to its new rect at once.
        expect(m.reflowFor("c")!.rect).toEqual(own(m, "c"));
        expect(m.reflowFor("d")!.rect).toEqual(own(m, "d"));
        expect(own(m, "d")).not.toEqual(before);
        expect(ghostIds(m)).toEqual(["a", "b"]);
        // The pass lasts from the second change.
        vi.advanceTimersByTime(SEED_MS + reflowSettleMs() - 1);
        expect(m.settlingPass()).toBeDefined();
        vi.advanceTimersByTime(1);
        expect(m.settlingPass()).toBeUndefined();
        expect(m.reflowFor("c")).toBeUndefined();
    });

    it("keeps the start of a pane whose first glide had not begun", () => {
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c"), leaf("d")));
        const before = own(m, "d");
        close(m, "a");
        close(m, "b");
        expect(m.reflowFor("d")!.rect).toEqual(before);
        vi.advanceTimersByTime(SEED_MS);
        expect(m.reflowFor("d")!.rect).toEqual(own(m, "d"));
    });

    it("puts the ghost of a pane that closes before its own glide began where the pane still stood", () => {
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c"), leaf("d")));
        const stood = m.additionalProps()["b"].rect;
        close(m, "a");
        expect(m.additionalProps()["b"].rect).not.toEqual(stood);
        close(m, "b");
        expect(m.settlingPass()!.ghosts[1]).toEqual({ id: "b", rect: stood });
    });

    it("puts the ghost of a pane that closes in the middle of its glide where its edges are drawn", () => {
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c"), leaf("d")));
        close(m, "a");
        vi.advanceTimersByTime(SEED_MS + 60);
        // The tile of b, part way from 300 to 0: its edges as the transition has them.
        const edges: Record<string, string> = { "--reflow-l": "120px", "--reflow-t": "0px", "--reflow-r": "480.5px", "--reflow-b": "800px" };
        vi.stubGlobal("document", { visibilityState: "visible", getElementById: (id: string) => (id === "b" ? { id } : null) });
        vi.stubGlobal("getComputedStyle", () => ({ getPropertyValue: (name: string) => edges[name] }));
        close(m, "b");
        expect(m.settlingPass()!.ghosts[1]).toEqual({ id: "b", rect: { left: 120, top: 0, width: 360.5, height: 800 } });
    });

    it("ends the pass when the layout is resized under it", () => {
        const m = modelOf(three());
        close(m, "c");
        vi.advanceTimersByTime(SEED_MS);
        container = { top: 0, left: 0, width: 1000, height: 800 };
        m.updateTree();
        expect(m.settlingPass()).toBeUndefined();
        expect(m.reflowFor("b")).toBeUndefined();
    });

    it("carries on through a layout pass that changes nothing", () => {
        const m = modelOf(three());
        close(m, "c");
        vi.advanceTimersByTime(SEED_MS);
        m.updateTree();
        expect(m.reflowFor("b")).toBeDefined();
    });
});

describe("a change that does not glide", () => {
    const snaps = (m: LayoutModel) => {
        expect(m.settlingPass()).toBeUndefined();
        expect(m.reflowFor("b")).toBeUndefined();
    };

    it("with the animations off: no entry and no ghost, the layout as it was before this", () => {
        setAnimationSettings({ "animation:preset": "off" });
        const m = modelOf(three());
        close(m, "c");
        snaps(m);
        expect(paneSizesHeld()).toBe(false);
    });

    it("with the live reflow's setting off", () => {
        setFlag(false);
        const m = modelOf(three());
        close(m, "c");
        snaps(m);
    });

    it("with more panes than the reflow moves", () => {
        const leaves = Array.from({ length: REFLOW_MAX_PANES + 1 }, (_, i) => leaf(i === 1 ? "b" : `p${i}`));
        const m = modelOf(row("r", 10, ...leaves));
        close(m, "p0");
        snaps(m);
    });

    it("while the window or a splitter is being resized", () => {
        const m = modelOf(three());
        m.isContainerResizing._set(true);
        close(m, "c");
        snaps(m);
    });

    it("while a pane is magnified", () => {
        const m = modelOf(three());
        m.treeState.magnifiedNodeId = "a";
        close(m, "c");
        snaps(m);
    });

    it("in a window tab that is not shown, whose layout stays mounted", () => {
        const m = modelOf(three());
        const hidden = { closest: (selector: string) => (selector === "[data-window-tab-hidden]" ? {} : null) };
        m.displayContainerRef.current = { getBoundingClientRect: () => container, ...hidden } as any;
        close(m, "c");
        snaps(m);
        expect(paneSizesHeld()).toBe(false);
    });

    it("while a pane's picture is being taken, and a picture that starts during a pass ends it", () => {
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c"), leaf("d")));
        const done = beginPaneCapture();
        close(m, "a");
        snaps(m);
        done();
        close(m, "c");
        expect(m.settlingPass()).toBeDefined();
        beginPaneCapture();
        snaps(m);
    });

    it("during a drag, and at the drop it commits in the same task as its release", () => {
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c"), leaf("d")));
        m.activeDrag._set(true);
        close(m, "a");
        expect(m.settlingPass()).toBeUndefined();
        // The release: the source's handler ends the drag, then the drop changes the tree.
        m.activeDrag._set(false);
        close(m, "c");
        expect(m.settlingPass()).toBeUndefined();
        expect(m.reflowFor("b")).toBeUndefined();
        // A task later the panes show the layout's own rects again, and a close glides.
        vi.advanceTimersByTime(1);
        close(m, "d");
        expect(m.reflowFor("b")).toBeDefined();
    });

    it("at the commit of a tear-off: the panes already show it", () => {
        const m = modelOf(three());
        m.activeDrag._set(true);
        m.activeDrag._set(false);
        commitTearOff(m, "a");
        vi.advanceTimersByTime(500);
        expect(m.reflowFor("b")!.rect).toMatchObject({ left: 0, width: 1200 });
        close(m, "a");
        expect(m.tearOffLayout()).toBeUndefined();
        snaps(m);
    });

    it("while a tab is being revealed: the panes that open into it are not seen arriving", () => {
        const m = modelOf(three());
        holdRevealGate("tab-1");
        open(m, "d");
        snaps(m);
        scheduleRevealLift();
        vi.advanceTimersByTime(5000);
        open(m, "e");
        expect(m.settlingPass()).toBeDefined();
    });

    it("when a pane is resized by a command: the arrangement is the same", () => {
        const m = modelOf(three());
        m.treeReducer({
            type: LayoutTreeActionType.ResizeNode,
            resizeOperations: [
                { nodeId: "a", size: 4 },
                { nodeId: "col", size: 16 },
            ],
        } as LayoutTreeResizeNodeAction);
        expect(own(m, "a").width).not.toBe(600);
        snaps(m);
    });

    it("when the layout's container changes size", () => {
        const m = modelOf(three());
        container = { top: 0, left: 0, width: 900, height: 600 };
        m.updateTree();
        expect(own(m, "a").width).toBe(450);
        snaps(m);
    });

    it("when an open pane is alone in the pass: nothing else moved, so nothing fades", () => {
        const m = modelOf(leaf("a"));
        close(m, "a");
        vi.advanceTimersByTime(1000);
        open(m, "z");
        expect(m.settlingPass()).toBeUndefined();
    });

    it("on the first layout of a tab", () => {
        const m = modelOf(three());
        expect(m.settlingPass()).toBeUndefined();
    });
});

describe("a pane opens", () => {
    it("moves the panes that make room from the rects they showed, and fades the new pane in", () => {
        const m = modelOf(three());
        const before = Object.fromEntries(["a", "b", "c"].map((id) => [id, own(m, id)]));
        open(m, "d");
        expect(m.leafs().some((l) => l.id === "d")).toBe(true);
        const pass = m.settlingPass()!;
        expect(pass.enters).toEqual(["d"]);
        expect(pass.ghosts).toEqual([]);
        // The new pane takes its rect at once: it is not one of the panes that glide.
        expect(pass.moves.has("d")).toBe(false);
        expect(m.reflowFor("d")).toBeUndefined();
        const moved = [...pass.moves.keys()];
        expect(moved.length).toBeGreaterThan(0);
        for (const id of moved) {
            expect(own(m, id)).not.toEqual(before[id]);
            expect(m.reflowFor(id)!.rect).toEqual(before[id]);
        }
    });

    it("glides when the new pane is put under another, whose leaf becomes the branch of the two", () => {
        // A full row (five panes): the insert goes under one of them, and that leaf's node changes its id in
        // place while the node that now holds its pane takes the old one.
        const m = modelOf(row("r", 10, leaf("a"), leaf("b"), leaf("c"), leaf("d"), leaf("e")));
        const before = Object.fromEntries(["a", "b", "c", "d", "e"].map((id) => [id, own(m, id)]));
        open(m, "f");
        const pass = m.settlingPass()!;
        expect(pass).toBeDefined();
        expect(pass.enters).toEqual(["f"]);
        expect(pass.ghosts).toEqual([]);
        const moved = [...pass.moves.keys()];
        expect(moved.length).toBe(1);
        expect(own(m, moved[0]).height).toBe(before[moved[0]].height / 2);
        expect(m.reflowFor(moved[0])!.rect).toEqual(before[moved[0]]);
    });

    it("keeps a pane that shrinks laid out at its larger size until the glide is over", () => {
        const m = modelOf(row("r", 10, leaf("a"), leaf("b")));
        m.treeReducer({
            type: LayoutTreeActionType.SplitHorizontal,
            targetNodeId: "b",
            newNode: leaf("n"),
            position: "after",
        } as LayoutTreeSplitHorizontalAction);
        const shrunk = [...m.settlingPass()!.moves.keys()].filter((id) => own(m, id).width < 600);
        expect(shrunk.length).toBeGreaterThan(0);
        vi.advanceTimersByTime(SEED_MS);
        // Gliding: the clip closes to the new rect over a tile still as wide as it was.
        for (const id of shrunk) expect(m.reflowFor(id)).toMatchObject({ rect: own(m, id), box: { width: 600 } });
        settle();
        expect(m.settlingPass()).toBeUndefined();
        for (const id of shrunk) expect(m.reflowFor(id)).toBeUndefined();
    });

    it("keeps the new pane fading when a second pane opens during its fade", () => {
        const m = modelOf(three());
        open(m, "d");
        vi.advanceTimersByTime(SEED_MS + 30);
        open(m, "e");
        expect([...m.settlingPass()!.enters].sort()).toEqual(["d", "e"]);
        settle();
        expect(m.settlingPass()).toBeUndefined();
    });

    it("lets a pane that opened and closes at once leave no fade behind", () => {
        const m = modelOf(three());
        open(m, "d");
        close(m, "d");
        expect(m.settlingPass()!.enters).toEqual([]);
        expect(ghostIds(m)).toEqual(["d"]);
    });
});

describe("a pane is minimized or restored", () => {
    it("glides the neighbours, and the minimized pane takes its chip at once", () => {
        const m = modelOf(three());
        const before = own(m, "b");
        minimize(m, "c");
        expect(m.minimizedNodeIds().has("c")).toBe(true);
        const pass = m.settlingPass()!;
        expect(pass.moves.has("c")).toBe(false);
        expect(pass.enters).toEqual([]);
        expect(pass.ghosts).toEqual([]);
        expect(own(m, "b").height).toBeGreaterThan(before.height);
        expect(m.reflowFor("b")!.rect).toEqual(before);
    });

    it("fades the restored pane in at its rect while the neighbours glide back", () => {
        const m = modelOf(three());
        minimize(m, "c");
        settle();
        const grown = own(m, "b");
        restore(m, "c");
        expect(m.minimizedNodeIds().has("c")).toBe(false);
        const pass = m.settlingPass()!;
        expect(pass.enters).toEqual(["c"]);
        expect(pass.moves.has("c")).toBe(false);
        expect(m.reflowFor("b")!.rect).toEqual(grown);
        settle();
        expect(m.settlingPass()).toBeUndefined();
    });
});

describe("a tear-off taken off after its drag ended", () => {
    /** A drag of "a" that left the window and was released there, its tear-off committed. */
    function releasedOutside(): LayoutModel {
        const m = modelOf(three());
        m.activeDrag._set(true);
        leaveWindow(m, "a");
        m.activeDrag._set(false);
        return m;
    }

    it("glides the panes back from where it left them to their own rects (Escape outside the window)", async () => {
        const m = releasedOutside();
        await Promise.resolve();
        const closed = shownPx(m.tearOffLayout()!.rects.b);
        expect(closed).toMatchObject({ left: 0, width: 1200 });
        returnToWindow(m);
        expect(m.tearOffLayout()).toBeUndefined();
        // The frame it opens on: still closed over the slot, laid out at the size it will have.
        expect(m.reflowFor("b")).toMatchObject({ rect: closed, leaving: false });
        vi.advanceTimersByTime(SEED_MS);
        expect(m.reflowFor("b")).toMatchObject({ rect: own(m, "b") });
        vi.advanceTimersByTime(reflowSettleMs());
        expect(m.reflowFor("b")).toBeUndefined();
        expect(m.settlingPass()).toBeUndefined();
    });

    it("snaps back with the animations off", async () => {
        setAnimationSettings({ "animation:preset": "off" });
        const m = releasedOutside();
        await Promise.resolve();
        returnToWindow(m);
        expect(m.settlingPass()).toBeUndefined();
        expect(m.reflowFor("b")).toBeUndefined();
    });

    it("opens no pass while the drag is live: its own reflow takes the panes back", () => {
        const m = modelOf(three());
        m.activeDrag._set(true);
        leaveWindow(m, "a");
        returnToWindow(m);
        expect(m.settlingPass()).toBeUndefined();
        expect(m.reflowFor("b")).toMatchObject({ rect: own(m, "b") });
    });

    it("opens none when the tree let the pane go: the panes already show the layout", async () => {
        const m = releasedOutside();
        await Promise.resolve();
        close(m, "a");
        expect(m.tearOffLayout()).toBeUndefined();
        expect(m.settlingPass()).toBeUndefined();
    });
});

describe("two panes are swapped", () => {
    it("glides each across to the other's place", () => {
        const m = modelOf(three());
        const before = { a: own(m, "a"), b: own(m, "b") };
        swap(m, "a", "b");
        expect(own(m, "a")).not.toEqual(before.a);
        const pass = m.settlingPass()!;
        expect([...pass.moves.keys()].sort()).toEqual(["a", "b"]);
        expect(pass.enters).toEqual([]);
        expect(pass.ghosts).toEqual([]);
        expect(m.reflowFor("a")!.rect).toEqual(before.a);
        expect(m.reflowFor("b")!.rect).toEqual(before.b);
        vi.advanceTimersByTime(SEED_MS);
        expect(m.reflowFor("a")!.rect).toEqual(own(m, "a"));
    });
});

describe("the plan of a pass", () => {
    const props = (rects: Record<string, Dimensions>) =>
        Object.fromEntries(Object.entries(rects).map(([id, rect]) => [id, { rect } as LayoutNodeAdditionalProps]));

    it("gives no entry to a pane that moved by less than a tile shows", () => {
        const m = modelOf(three());
        const at = (left: number) => ({ left, top: 0, width: 300, height: 800 });
        const pass = planSettlingPass(
            m,
            [leaf("a"), leaf("b"), leaf("c")],
            props({ a: at(0), b: at(300.2), c: at(600) }),
            [leaf("a"), leaf("b")],
            props({ a: at(0), b: at(300.6) })
        );
        expect([...pass!.moves.keys()]).toEqual([]);
        expect(pass!.ghosts.map((ghost) => ghost.id)).toEqual(["c"]);
    });

    it("is none when a pane left and another came in the same change", () => {
        const m = modelOf(three());
        const rect = { left: 0, top: 0, width: 600, height: 800 };
        expect(planSettlingPass(m, [leaf("a"), leaf("b")], props({ a: rect, b: rect }), [leaf("a"), leaf("d")], props({ a: rect, d: rect }))).toBeUndefined();
    });
});
