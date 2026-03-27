// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { newLayoutNode } from "@/layout/lib/layoutNode";
import {
    FlexDirection,
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
});
