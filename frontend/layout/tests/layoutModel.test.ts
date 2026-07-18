// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { newLayoutNode } from "@/layout/lib/layoutNode";
import {
    FlexDirection,
    LayoutNode,
    LayoutTreeActionType,
    LayoutTreeInsertNodeAction,
    LayoutTreeSplitHorizontalAction,
    LayoutTreeSetPendingAction,
    LayoutTreeCommitPendingAction,
} from "@/layout/lib/types";
import type { SignalAtom } from "@/util/util";

// Mock layoutState store keyed by oref
const layoutStateSignals = new Map<string, SignalAtom<LayoutState>>();

function makeLayoutStateSignal(oid: string): SignalAtom<LayoutState> {
    const [get, set] = createSignal<LayoutState>({
        otype: "layout",
        oid,
        version: 1,
        meta: {},
        rootnode: undefined,
        magnifiednodeid: undefined,
        focusednodeid: undefined,
        leaforder: undefined,
        pendingbackendactions: undefined,
    });
    const atom = () => get();
    (atom as any)._set = set;
    return atom as unknown as SignalAtom<LayoutState>;
}

vi.mock("@/app/store/global", () => {
    return {
        WOS: {
            makeORef: (_otype: string, oid: string) => oid,
            getWaveObjectAtom: (oid: string) => {
                if (!layoutStateSignals.has(oid)) {
                    layoutStateSignals.set(oid, makeLayoutStateSignal(oid));
                }
                return layoutStateSignals.get(oid);
            },
            getObjectValue: (oref: string) => {
                const sig = layoutStateSignals.get(oref);
                return sig ? sig() : undefined;
            },
            setObjectValue: (value: any) => {
                const oref = `${value.otype}:${value.oid}`;
                const sig = layoutStateSignals.get(value.oid) ?? layoutStateSignals.get(oref);
                if (sig) sig._set(value);
            },
        },
        getSettingsKeyAtom: () => {
            const [get] = createSignal(0.75);
            return get;
        },
        globalStore: {
            get: (accessor: any) => (typeof accessor === "function" ? accessor() : undefined),
            set: (setter: any, value: any) => {
                if (setter && typeof setter._set === "function") setter._set(value);
                else if (typeof setter === "function") setter(value);
            },
        },
    };
});

function createLayoutModel(): LayoutModel {
    const [getTab] = createSignal<Tab>({
        otype: "tab",
        oid: "tab-1",
        version: 1,
        meta: {},
        name: "Test Tab",
        layoutstate: "layout-1",
        blockids: [],
    });
    const model = new LayoutModel(getTab);
    model.getBoundingRect = () => ({
        top: 0,
        left: 0,
        width: 800,
        height: 600,
    });
    model.displayContainerRef.current = {
        getBoundingClientRect: () => ({
            top: 0,
            left: 0,
            width: 800,
            height: 600,
        }),
    } as any;
    return model;
}

describe("LayoutModel", () => {
    beforeEach(() => {
        layoutStateSignals.clear();
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it("creates a root node and focuses it when inserting the first block", () => {
        const model = createLayoutModel();
        const node = newLayoutNode(undefined, undefined, undefined, { blockId: "block-1" });

        model.treeReducer({
            type: LayoutTreeActionType.InsertNode,
            node,
            magnified: false,
            focused: true,
        } as LayoutTreeInsertNodeAction);

        expect(model.treeState.rootNode?.data?.blockId).toBe("block-1");
        expect(model.treeState.focusedNodeId).toBe(node.id);
        expect(model.treeState.rootNode?.children).toBeUndefined();
    });

    it("splits an existing node horizontally and focuses the new block", () => {
        const model = createLayoutModel();
        const first = newLayoutNode(undefined, undefined, undefined, { blockId: "left" });
        model.treeReducer({
            type: LayoutTreeActionType.InsertNode,
            node: first,
            magnified: false,
            focused: true,
        } as LayoutTreeInsertNodeAction);

        const second = newLayoutNode(undefined, undefined, undefined, { blockId: "right" });
        model.treeReducer(
            {
                type: LayoutTreeActionType.SplitHorizontal,
                targetNodeId: model.treeState.rootNode!.id,
                newNode: second,
                position: "after",
                focused: true,
            } as LayoutTreeSplitHorizontalAction,
            false,
        );

        const root = model.treeState.rootNode!;
        expect(root.flexDirection).toBe(FlexDirection.Row);
        expect(root.children).toHaveLength(2);
        expect(root.children![0].data?.blockId).toBe("left");
        expect(root.children![1].data?.blockId).toBe("right");
        expect(model.treeState.focusedNodeId).toBe(second.id);
    });

    it("commits pending insert actions through the pending action queue", () => {
        const model = createLayoutModel();
        const first = newLayoutNode(undefined, undefined, undefined, { blockId: "primary" });
        model.treeReducer({
            type: LayoutTreeActionType.InsertNode,
            node: first,
            magnified: false,
            focused: true,
        } as LayoutTreeInsertNodeAction);

        const pending = newLayoutNode(undefined, undefined, undefined, { blockId: "secondary" });
        model.treeReducer(
            {
                type: LayoutTreeActionType.SetPendingAction,
                action: {
                    type: LayoutTreeActionType.InsertNode,
                    node: pending,
                    magnified: false,
                    focused: true,
                } as LayoutTreeInsertNodeAction,
            } as LayoutTreeSetPendingAction,
            false,
        );

        model.treeReducer({ type: LayoutTreeActionType.CommitPendingAction } as LayoutTreeCommitPendingAction, false);

        // Advance timers to allow throttled signal to update
        vi.advanceTimersByTime(20);

        const root = model.treeState.rootNode!;
        const leafBlocks = root.children
            ? root.children.map((child) => child.data?.blockId)
            : [root.data?.blockId];
        expect(leafBlocks).toContain("primary");
        expect(leafBlocks).toContain("secondary");

        // After commit, the pending action should be cleared
        const pendingAction = model.pendingTreeAction.throttledValueAtom();
        expect(pendingAction).toBeUndefined();
    });

    // Regression / feature restoration: the exact user-reported repro
    // (docs/retro/retro-minimize-display-mode-lost-slip-requirement-2026-07-17.md)
    // — minimize cpu then swarm in the default agent/cpu/swarm layout. Per
    // SPEC_PANE_MINIMIZE_COLUMN_DISSOLVE_2026_06_27.md, right_col (now fully
    // minimized) must dock its stacked headers onto agent — not render as a
    // separate narrow column beside it — with agent's content area shrinking
    // to make room and absorbing right_col's freed width.
    it("docks a fully-minimized column's header stack onto its Row sibling instead of a separate slot", () => {
        const model = createLayoutModel();
        const agent = newLayoutNode(FlexDirection.Column, 5, undefined, { blockId: "agent" });
        const cpu = newLayoutNode(FlexDirection.Row, 2, undefined, { blockId: "cpu" });
        const swarm = newLayoutNode(FlexDirection.Row, 8, undefined, { blockId: "swarm" });
        const rightCol = newLayoutNode(FlexDirection.Column, 5, [cpu, swarm]);
        const root = newLayoutNode(FlexDirection.Row, 10, [agent, rightCol]);
        model.treeState.rootNode = root;

        model.updateTree();
        const beforeProps = model.additionalProps();
        expect(beforeProps[rightCol.id].rect.width).toBeGreaterThan(0);
        expect(beforeProps[agent.id].rect.width).toBeGreaterThan(0);

        cpu.minimized = true;
        model.updateTree();
        swarm.minimized = true;
        model.updateTree();

        const gap = model.gapSizePx();
        const headerH = 33; // HeaderHeightPx
        const props = model.additionalProps();

        // agent's own slot absorbed ALL of rightCol's freed width — the
        // reported bug (a separate ~180px-wide narrow strip next to agent)
        // must not exist: agent's rect now spans the full row width.
        expect(props[agent.id].rect.width).toBeCloseTo(800, 0);

        // agent's content area shrank + shifted down to leave room for the
        // 2-chip stack docked above it (cpu, then swarm).
        const dockedHeight = 2 * (headerH + gap);
        expect(props[agent.id].rect.top).toBeCloseTo(dockedHeight, 0);
        expect(props[agent.id].rect.height).toBeCloseTo(600 - dockedHeight, 0);

        // rightCol itself renders as the chip-stack overlay, pinned to the
        // TOP of agent's original slot — same width as agent, not its own
        // separate narrow column.
        expect(props[rightCol.id].rect.top).toBeCloseTo(0, 0);
        expect(props[rightCol.id].rect.width).toBeCloseTo(800, 0);
        expect(props[rightCol.id].rect.height).toBeCloseTo(dockedHeight, 0);

        // The two chips inside rightCol's (now correctly clamped) box stack
        // with zero dead space between or below them.
        expect(props[cpu.id].rect.top).toBeCloseTo(0, 0);
        expect(props[cpu.id].rect.height).toBeCloseTo(headerH + gap, 0);
        expect(props[swarm.id].rect.top).toBeCloseTo(headerH + gap, 0);
        expect(props[swarm.id].rect.height).toBeCloseTo(headerH + gap, 0);

        // No resize handle survives between agent and the now-docked rightCol.
        expect(props[root.id].resizeHandles).toHaveLength(0);
    });

    // End-to-end regression for the exact user-reported corruption
    // (2026-07-18): a blind InsertNode (e.g. a new pane created via the "+"
    // button or an agent creating a terminal) whose heuristic target
    // resolves onto a minimized leaf must not promote that leaf into a
    // branch — the promotion left `minimized` stranded on the branch
    // (violating the doctor's I2) while the id-inheriting intermediate
    // child silently lost it, and the separately-tracked minimizedNodeIds
    // set kept the stale id — so an EXPANDED pane showed a Restore button.
    it("InsertNode never promotes a minimized leaf, and minimizedNodeIds cannot desync from the tree", () => {
        const model = createLayoutModel();
        // Root must be AT capacity (5 children, DEFAULT_MAX_CHILDREN=5) so it
        // is excluded as an insert candidate itself — otherwise root always
        // wins regardless of any leaf's minimize state and this test would
        // pass trivially without ever exercising the corrupted path. With 5
        // same-depth leaf candidates tied on score, the heuristic's (stable)
        // sort picks whichever was visited first — reverse child order means
        // the LAST array element wins the tie; put the minimized leaf there
        // so, absent the fix, it's exactly what would get promoted.
        const leaves = ["a", "b", "c", "d", "minimized"].map((id) =>
            newLayoutNode(FlexDirection.Column, 5, undefined, { blockId: id })
        );
        const minimized = leaves[4];
        minimized.minimized = true;
        const root = newLayoutNode(FlexDirection.Row, 10, leaves);
        model.treeState.rootNode = root;
        model.updateTree();
        expect(model.additionalProps()).toHaveProperty(minimized.id);

        const newBlock = newLayoutNode(undefined, undefined, undefined, { blockId: "new" });
        model.treeReducer({
            type: LayoutTreeActionType.InsertNode,
            node: newBlock,
            magnified: false,
            focused: false,
        } as LayoutTreeInsertNodeAction);

        // The minimized leaf must still be a LEAF, still minimized, still
        // exactly where it was — untouched by the insert.
        const stillMinimized = model.treeState.rootNode!.children!.find((c) => c.data?.blockId === "minimized");
        expect(stillMinimized).toBeDefined();
        expect(stillMinimized!.children).toBeUndefined();
        expect(stillMinimized!.minimized).toBe(true);

        // No node anywhere in the tree is a branch carrying `minimized` (the
        // exact corruption) — walk the whole tree and check.
        function walk(node: LayoutNode | undefined, assertNotCorrupted: (n: LayoutNode) => void) {
            if (!node) return;
            assertNotCorrupted(node);
            node.children?.forEach((c) => walk(c, assertNotCorrupted));
        }
        walk(model.treeState.rootNode, (n) => {
            if (n.children?.length) expect(n.minimized).toBeUndefined();
        });

        // minimizedNodeIds (derived fresh by updateTree, which treeReducer
        // just triggered) exactly matches reality: the minimized leaf and
        // nothing else.
        expect(model.minimizedNodeIds()).toEqual(new Set([stillMinimized!.id]));
    });
});
