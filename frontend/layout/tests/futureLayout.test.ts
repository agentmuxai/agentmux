// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The drop preview is the drop: for every zone of every dragged pane over every other pane, on a set of
 * realistic trees, the ghost (the moved pane's rect in `futureLayoutOf`) and the wireframe (every other
 * pane's rect) equal the rects the layout has once the same action is dropped on a model of its own.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { futureLayoutOf } from "@/layout/lib/futureLayout";
import { pillStandIn } from "@/layout/lib/paneBodyDrop";
import { applyLayoutAction } from "@/layout/lib/layoutIntentActions";
import { computeMoveNode } from "@/layout/lib/layoutTree";
import { walkNodes } from "@/layout/lib/layoutNode";
import { DropDirection, FlexDirection, LayoutTreeActionType, type LayoutNode } from "@/layout/lib/types";
import type { SignalAtom } from "@/util/util";

const rows = new Map<string, SignalAtom<LayoutState>>();
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
    getSettingsKeyAtom: () => createSignal(0.75)[0],
    globalStore: { get: () => undefined, set: () => {} },
}));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: { requestNodeFocus: () => {}, refocusNode: () => {}, claimFocusOnMount: () => {}, ensureSelectionFocused: () => {} },
}));

const leaf = (id: string, size = 10): LayoutNode => ({ id, size, data: { blockId: id } }) as LayoutNode;
const row = (id: string, size: number, ...children: LayoutNode[]): LayoutNode =>
    ({ id, size, flexDirection: FlexDirection.Row, children }) as LayoutNode;
const col = (id: string, size: number, ...children: LayoutNode[]): LayoutNode =>
    ({ id, size, flexDirection: FlexDirection.Column, children }) as LayoutNode;

/** A set of trees from two to eight panes: nested splits, uneven sizes. */
const TREES: Record<string, () => LayoutNode> = {
    "two panes side by side": () => row("r", 10, leaf("a"), leaf("b")),
    "two panes stacked": () => col("r", 10, leaf("a"), leaf("b")),
    "a pane beside a stack of two": () => row("r", 10, leaf("a"), col("col", 10, leaf("b"), leaf("c"))),
    "two rows in a column, uneven": () =>
        col("r", 10, row("r1", 6, leaf("a", 7), leaf("b", 13)), row("r2", 14, leaf("c"), leaf("d"))),
    "three columns, one nested": () =>
        row("r", 10, leaf("a", 5), col("col", 12, leaf("b"), row("r2", 10, leaf("c"), leaf("d"))), leaf("e", 8)),
    "eight panes": () =>
        row(
            "r",
            10,
            col("c1", 7, leaf("a"), leaf("b"), leaf("c")),
            col("c2", 10, leaf("d"), leaf("e")),
            row("r2", 13, leaf("f"), col("c3", 10, leaf("g"), leaf("h")))
        ),
};

function modelOf(tree: LayoutNode): LayoutModel {
    const [tab] = createSignal<Tab>({ otype: "tab", oid: "tab-1", version: 1, meta: {}, name: "t", layoutstate: "layout-1", blockids: [] });
    const m = new LayoutModel(tab);
    m.getBoundingRect = () => ({ top: 0, left: 0, width: 1200, height: 800 });
    m.displayContainerRef.current = { getBoundingClientRect: () => ({ top: 0, left: 0, width: 1200, height: 800 }) } as any;
    m.treeState.rootNode = structuredClone(tree);
    m.updateTree();
    return m;
}

const leafIds = (tree: LayoutNode): string[] => {
    const ids: string[] = [];
    walkNodes(tree, (n) => void (!n.children?.length && ids.push(n.id)));
    return ids;
};

const DIRECTIONS = [
    DropDirection.Top,
    DropDirection.Right,
    DropDirection.Bottom,
    DropDirection.Left,
    DropDirection.OuterTop,
    DropDirection.OuterRight,
    DropDirection.OuterBottom,
    DropDirection.OuterLeft,
    DropDirection.Center,
];

const near = (a: Dimensions, b: Dimensions) =>
    (["top", "left", "width", "height"] as const).every((k) => Math.abs(a[k] - b[k]) <= 1);

/** Every (dragged, target, zone) the reducer accepts on `tree`, as the pending action it computes. */
function drops(tree: LayoutNode) {
    const probe = modelOf(tree);
    const out: { name: string; dragged: string; action: ReturnType<typeof computeMoveNode> }[] = [];
    for (const dragged of leafIds(tree)) {
        for (const target of leafIds(tree)) {
            if (target === dragged) continue;
            for (const direction of DIRECTIONS) {
                const action = computeMoveNode(probe.treeState, {
                    type: LayoutTreeActionType.ComputeMove,
                    nodeId: target,
                    nodeToMoveId: dragged,
                    direction,
                });
                if (action) out.push({ name: `${dragged} over ${target}, ${DropDirection[direction]}`, dragged, action });
            }
        }
    }
    return out;
}

describe("the drop preview is the drop", () => {
    beforeEach(() => rows.clear());

    for (const [name, make] of Object.entries(TREES)) {
        it(`${name}: the ghost and every pane's wireframe equal the layout after the drop`, () => {
            const cases = drops(make());
            expect(cases.length).toBeGreaterThan(6);
            const wrong: string[] = [];
            for (const { name: zone, dragged, action } of cases) {
                const shown = modelOf(make());
                const future = futureLayoutOf(shown, action!);
                // What the drop leaves, on a model of its own: the action dropped, the layout laid out.
                const dropped = modelOf(make());
                applyLayoutAction(dropped.treeState, action!);
                dropped.updateTree();
                const after = (id: string) => dropped.additionalProps()[id]?.rect;
                if (!future) {
                    wrong.push(`${zone}: no preview`);
                    continue;
                }
                const ghost = future.rects[dragged];
                if (!ghost || !after(dragged) || !near(ghost, after(dragged)!)) wrong.push(`${zone}: ghost`);
                // Every pane the preview outlines is where the drop puts it, and none is left out.
                const droppedLeaves = leafIds(dropped.treeState.rootNode!);
                if (droppedLeaves.length !== future.leafIds.length) wrong.push(`${zone}: pane count`);
                for (const id of droppedLeaves.filter((l) => l !== dragged)) {
                    const mine = future.rects[id];
                    if (!mine || !after(id) || !near(mine, after(id)!)) wrong.push(`${zone}: wireframe ${id}`);
                }
                // The preview leaves the layout it was computed from untouched.
                expect(shown.treeState.rootNode).toEqual(modelOf(make()).treeState.rootNode);
            }
            expect(wrong).toEqual([]);
        });
    }
});

/**
 * Phase 0 of the live reflow (SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md §5, §9): the preview's layout work (a
 * copy of the tree, the reducer, the layout pass) runs once per armed candidate, a few times a second. Its
 * budget is 4 ms for eight panes on a developer machine; the assertion is ten times looser so a slow CI runner
 * can't fail it, and the measured means are printed for the record.
 */
describe("the drop preview is cheap enough to run per armed candidate", () => {
    beforeEach(() => rows.clear());

    it("stays well under budget on two to eight panes", () => {
        const measured: string[] = [];
        const slow: string[] = [];
        for (const [name, make] of Object.entries(TREES)) {
            const model = modelOf(make());
            const ids = leafIds(make());
            // The last move the reducer accepts on this tree: a pane moved far from where it is.
            const accepted = drops(make());
            const action = accepted[accepted.length - 1]?.action;
            expect(action).toBeTruthy();
            // Warm up, then time the mean of many runs: one run is below the clock's resolution.
            for (let i = 0; i < 10; i++) futureLayoutOf(model, action!);
            const runs = 200;
            const start = performance.now();
            for (let i = 0; i < runs; i++) futureLayoutOf(model, action!);
            const mean = (performance.now() - start) / runs;
            measured.push(`${name} (${ids.length} panes): ${mean.toFixed(3)} ms`);
            if (mean > 40) slow.push(`${name}: ${mean.toFixed(2)} ms`);
        }
        console.info(`[futureLayoutOf] mean per preview:\n  ${measured.join("\n  ")}`);
        expect(slow).toEqual([]);
    });
});

/**
 * The live reflow matches a current pane to its future rect by blockId (SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md
 * §6): a pane keeps its node id through the preview, but a split can rename its parent, so the block is the key.
 */
describe("every current pane has exactly one future rect, by block", () => {
    beforeEach(() => rows.clear());

    for (const [name, make] of Object.entries(TREES)) {
        it(`${name}: each pane's block is in the preview once, with a rect`, () => {
            const wrong: string[] = [];
            for (const { name: zone, action } of drops(make())) {
                const model = modelOf(make());
                const future = futureLayoutOf(model, action!);
                if (!future) {
                    wrong.push(`${zone}: no preview`);
                    continue;
                }
                const blocks = future.leafIds.map((id) => future.leafBlockIds[id]);
                for (const leaf of model.leafs()) {
                    const block = leaf.data?.blockId;
                    const mine = blocks.filter((b) => b === block);
                    if (mine.length !== 1) wrong.push(`${zone}: ${leaf.id} appears ${mine.length} times`);
                    const id = future.leafIds.find((l) => future.leafBlockIds[l] === block);
                    if (!id || !future.rects[id]) wrong.push(`${zone}: ${leaf.id} has no rect`);
                }
            }
            expect(wrong).toEqual([]);
        });
    }
});

describe("a tab pill's ghost has an extent", () => {
    beforeEach(() => rows.clear());

    for (const [name, make] of Object.entries(TREES)) {
        it(`${name}: the pill's stand-in is laid out as a pane, not a line, beside every pane`, () => {
            const probe = modelOf(make());
            const stand = pillStandIn("pill-block");
            const thin: string[] = [];
            let seen = 0;
            for (const target of leafIds(make())) {
                for (const direction of [DropDirection.Top, DropDirection.Right, DropDirection.Bottom, DropDirection.Left]) {
                    const action = computeMoveNode(probe.treeState, {
                        type: LayoutTreeActionType.ComputeMove,
                        nodeId: target,
                        nodeToMoveId: stand.id,
                        direction,
                        nodeToMove: stand,
                    } as any);
                    if (!action) continue;
                    seen++;
                    const ghost = futureLayoutOf(modelOf(make()), action)?.rects[stand.id];
                    if (!ghost || ghost.width < 40 || ghost.height < 40) thin.push(`${target} ${DropDirection[direction]}: ${JSON.stringify(ghost)}`);
                }
            }
            expect(seen).toBeGreaterThan(3);
            expect(thin).toEqual([]);
        });
    }
});
