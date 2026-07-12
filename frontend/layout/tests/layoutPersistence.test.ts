// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for processPendingBackendActions — specifically the Phase 4b
// SplitHorizontal and SplitVertical action routes used by the floating-pane
// redock ghost-size fix.

import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { findNodeByBlockId, newLayoutNode } from "@/layout/lib/layoutNode";
import { LayoutTreeActionType, LayoutTreeInsertNodeAction } from "@/layout/lib/types";
import { processPendingBackendActions, pruneDanglingLeaves, markBlockRecentlyCreated } from "@/layout/lib/layoutPersistence";
import type { SignalAtom } from "@/util/util";

// -- Mock store (mirrors layoutModel.test.ts) ----------------------------------

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

// -- Helpers ------------------------------------------------------------------

function createLayoutModel(): LayoutModel {
    const [getTab] = createSignal<Tab>({
        otype: "tab",
        oid: "tab-1",
        version: 1,
        meta: {},
        name: "Test",
        layoutstate: "layout-1",
        // Every block id these tests mount or insert-via-action. In
        // production Tab.blockids is reducer-owned truth and
        // pruneDanglingLeaves (layoutPersistence.ts) removes any leaf
        // whose block isn't in it — a stub that owns nothing would get
        // every test leaf pruned right after insertion.
        blockids: ["existing", "moved-block", "new-block", "dup-block"],
    });
    const m = new LayoutModel(getTab);
    m.getBoundingRect = () => ({ top: 0, left: 0, width: 800, height: 600 });
    m.displayContainerRef.current = {
        getBoundingClientRect: () => ({ top: 0, left: 0, width: 800, height: 600 }),
    } as any;
    return m;
}

function insertBlock(model: LayoutModel, blockId: string) {
    const node = newLayoutNode(undefined, undefined, undefined, { blockId });
    model.treeReducer({ type: LayoutTreeActionType.InsertNode, node, magnified: false, focused: true } as LayoutTreeInsertNodeAction);
    return node;
}

function setPendingActions(model: LayoutModel, actions: any[]) {
    const waveObj = model.getter(model.waveObjectAtom) as LayoutState;
    const sig = layoutStateSignals.get(waveObj.oid);
    if (sig) sig._set({ ...waveObj, pendingbackendactions: actions });
}

function findBlock(model: LayoutModel, blockId: string) {
    if (!model.treeState.rootNode) return null;
    return findNodeByBlockId(model.treeState.rootNode, blockId);
}

// -- Tests --------------------------------------------------------------------

describe("processPendingBackendActions — Phase 4b Split routes", () => {
    beforeEach(() => {
        layoutStateSignals.clear();
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it("SplitHorizontal after — places new block to the right of target", async () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.SplitHorizontal,
            actionid: "action-h-after",
            blockid: "new-block",
            targetblockid: "existing",
            position: "after",
            nodesize: undefined,
            focused: true,
            magnified: false,
            ephemeral: false,
        }]);

        await processPendingBackendActions(model);

        const root = model.treeState.rootNode!;
        expect(root.children).toHaveLength(2);
        expect(root.children![0].data?.blockId).toBe("existing");
        expect(root.children![1].data?.blockId).toBe("new-block");
    });

    it("SplitHorizontal before — places new block to the left of target", async () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.SplitHorizontal,
            actionid: "action-h-before",
            blockid: "new-block",
            targetblockid: "existing",
            position: "before",
            nodesize: undefined,
            focused: true,
            magnified: false,
            ephemeral: false,
        }]);

        await processPendingBackendActions(model);

        const root = model.treeState.rootNode!;
        expect(root.children).toHaveLength(2);
        expect(root.children![0].data?.blockId).toBe("new-block");
        expect(root.children![1].data?.blockId).toBe("existing");
    });

    it("SplitVertical after — places new block below target", async () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.SplitVertical,
            actionid: "action-v-after",
            blockid: "new-block",
            targetblockid: "existing",
            position: "after",
            nodesize: undefined,
            focused: true,
            magnified: false,
            ephemeral: false,
        }]);

        await processPendingBackendActions(model);

        const root = model.treeState.rootNode!;
        expect(root.children).toHaveLength(2);
        expect(root.children![0].data?.blockId).toBe("existing");
        expect(root.children![1].data?.blockId).toBe("new-block");
    });

    it("SplitVertical before — places new block above target", async () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.SplitVertical,
            actionid: "action-v-before",
            blockid: "new-block",
            targetblockid: "existing",
            position: "before",
            nodesize: undefined,
            focused: true,
            magnified: false,
            ephemeral: false,
        }]);

        await processPendingBackendActions(model);

        const root = model.treeState.rootNode!;
        expect(root.children).toHaveLength(2);
        expect(root.children![0].data?.blockId).toBe("new-block");
        expect(root.children![1].data?.blockId).toBe("existing");
    });

    it("SplitHorizontal with nodesize — outer-direction drop assigns specified size to new node", async () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.SplitHorizontal,
            actionid: "action-h-size",
            blockid: "new-block",
            targetblockid: "existing",
            position: "before",
            nodesize: 3,
            focused: true,
            magnified: false,
            ephemeral: false,
        }]);

        await processPendingBackendActions(model);

        const root = model.treeState.rootNode!;
        expect(root.children).toHaveLength(2);
        expect(root.children![0].size).toBe(3);
    });

    it("SplitHorizontal with nodesizefraction — sizes derive from the target's CURRENT size, not a default", async () => {
        // ANALYSIS_FLOATING_PANE_GHOST_LANDING_DISCONNECT_2026_07_04.md Root Cause 1:
        // a target that isn't at DefaultNodeSize (e.g. user-resized) must still land
        // at the ghost's exact ratio. Simulate a resized target (size 20, not 10).
        const model = createLayoutModel();
        const target = insertBlock(model, "existing");
        target.size = 20;

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.SplitHorizontal,
            actionid: "action-h-fraction",
            blockid: "new-block",
            targetblockid: "existing",
            position: "before",
            nodesize: undefined,
            nodesizefraction: 0.2,
            focused: true,
            magnified: false,
            ephemeral: false,
        }]);

        await processPendingBackendActions(model);

        const root = model.treeState.rootNode!;
        expect(root.children).toHaveLength(2);
        expect(root.children![0].data?.blockId).toBe("new-block");
        expect(root.children![0].size).toBe(4); // 0.2 * 20
        expect(root.children![1].size).toBe(16); // 20 - 4
    });

    it("SplitHorizontal logs error and no-ops when targetblockid not found", async () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");
        const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.SplitHorizontal,
            actionid: "action-h-miss",
            blockid: "new-block",
            targetblockid: "does-not-exist",
            position: "after",
            nodesize: undefined,
            focused: true,
            magnified: false,
            ephemeral: false,
        }]);

        await processPendingBackendActions(model);

        // No split happened — "existing" is still the only leaf
        expect(findBlock(model, "new-block")).toBeFalsy();
        expect(consoleSpy).toHaveBeenCalled();
        consoleSpy.mockRestore();
    });

    it("SplitVertical logs error when position is invalid", async () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");
        const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.SplitVertical,
            actionid: "action-v-badpos",
            blockid: "new-block",
            targetblockid: "existing",
            position: "center",  // invalid — only "before"/"after" are accepted
            nodesize: undefined,
            focused: true,
            magnified: false,
            ephemeral: false,
        }]);

        await processPendingBackendActions(model);

        expect(findBlock(model, "new-block")).toBeFalsy();
        expect(consoleSpy).toHaveBeenCalled();
        consoleSpy.mockRestore();
    });

    it("DeleteNode from a backend action removes node without calling closeNode (R1 regression guard)", async () => {
        // R1 (#1681): backend DeleteNode actions must use treeReducer directly —
        // NOT closeNode — because DeleteNode in this context means "block moved",
        // and calling closeNode would trigger DeleteBlock, destroying the moved block.
        const model = createLayoutModel();
        insertBlock(model, "moved-block");

        const closeNodeSpy = vi.spyOn(model, "closeNode");

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.DeleteNode,
            actionid: "action-del",
            blockid: "moved-block",
            targetblockid: "",
            position: "",
            nodesize: undefined,
            focused: false,
            magnified: false,
            ephemeral: false,
        }]);

        await processPendingBackendActions(model);

        // The block node is no longer in the tree
        expect(findBlock(model, "moved-block")).toBeNull();
        // closeNode was NOT called — which means DeleteBlock was NOT triggered
        expect(closeNodeSpy).not.toHaveBeenCalled();
    });

    it("actions with duplicate actionid are processed only once (idempotent)", async () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");

        const ACTION_ID = "dedup-action";
        setPendingActions(model, [
            {
                actiontype: LayoutTreeActionType.SplitHorizontal,
                actionid: ACTION_ID,
                blockid: "new-block",
                targetblockid: "existing",
                position: "after",
                focused: true,
                magnified: false,
                ephemeral: false,
            },
            {
                actiontype: LayoutTreeActionType.SplitHorizontal,
                actionid: ACTION_ID,  // same id — must be skipped
                blockid: "dup-block",
                targetblockid: "existing",
                position: "after",
                focused: true,
                magnified: false,
                ephemeral: false,
            },
        ]);

        await processPendingBackendActions(model);

        // "new-block" was inserted, "dup-block" was not (duplicate action id)
        expect(findBlock(model, "new-block")).toBeTruthy();
        expect(findBlock(model, "dup-block")).toBeFalsy();
    });
});

// -- pruneDanglingLeaves --------------------------------------------------------
//
// The stale-tree-resurrection healer (SPEC_PANE_DRAG_TO_TAB addendum A2):
// a leaf whose block is NOT in the tab's reducer-owned blockids is a
// dangling reference (renders a block another tab owns, double-mounting
// it) and must be removed; owned leaves are untouched.

describe("pruneDanglingLeaves", () => {
    it("removes leaves for blocks the tab does not own, keeps owned ones", () => {
        const model = createLayoutModel();
        insertBlock(model, "existing"); // in the stub tab's blockids
        // A dangling leaf — "ghost-block" is NOT in the stub's blockids.
        const ghost = newLayoutNode(undefined, undefined, undefined, { blockId: "ghost-block" });
        model.treeReducer({
            type: LayoutTreeActionType.InsertNode,
            node: ghost,
            magnified: false,
            focused: false,
        } as LayoutTreeInsertNodeAction);
        expect(findBlock(model, "ghost-block")).toBeTruthy();

        pruneDanglingLeaves(model);

        expect(findBlock(model, "ghost-block")).toBeFalsy();
        expect(findBlock(model, "existing")).toBeTruthy();
    });

    it("no-ops when every leaf is owned", () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");
        insertBlock(model, "moved-block");
        pruneDanglingLeaves(model);
        expect(findBlock(model, "existing")).toBeTruthy();
        expect(findBlock(model, "moved-block")).toBeTruthy();
    });

    // Review finding on PR #2105 (P1): createBlock/createBlockSplit* insert the
    // new leaf into the local tree BEFORE tab.blockids catches up. A leaf for a
    // block marked via markBlockRecentlyCreated must survive a prune that runs
    // in that window, and must become prunable again once the mark expires.
    it("does not prune a leaf for a block marked recently created", () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");
        const freshLeaf = newLayoutNode(undefined, undefined, undefined, { blockId: "fresh-not-yet-owned" });
        model.treeReducer({
            type: LayoutTreeActionType.InsertNode,
            node: freshLeaf,
            magnified: false,
            focused: false,
        } as LayoutTreeInsertNodeAction);
        markBlockRecentlyCreated("fresh-not-yet-owned", 1_000_000);

        const originalNow = Date.now;
        Date.now = () => 1_000_500; // 500ms later — well inside the 3s grace window
        try {
            pruneDanglingLeaves(model);
        } finally {
            Date.now = originalNow;
        }

        expect(findBlock(model, "fresh-not-yet-owned")).toBeTruthy();
        expect(findBlock(model, "existing")).toBeTruthy();
    });

    it("prunes a leaf once its recently-created mark has expired", () => {
        const model = createLayoutModel();
        const staleLeaf = newLayoutNode(undefined, undefined, undefined, { blockId: "was-fresh-now-stale" });
        model.treeReducer({
            type: LayoutTreeActionType.InsertNode,
            node: staleLeaf,
            magnified: false,
            focused: false,
        } as LayoutTreeInsertNodeAction);
        markBlockRecentlyCreated("was-fresh-now-stale", 1_000_000);

        const originalNow = Date.now;
        Date.now = () => 1_010_000; // 10s later — past the 3s grace window
        try {
            pruneDanglingLeaves(model);
        } finally {
            Date.now = originalNow;
        }

        expect(findBlock(model, "was-fresh-now-stale")).toBeFalsy();
    });
});
