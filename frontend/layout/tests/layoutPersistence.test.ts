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
import { processPendingBackendActions } from "@/layout/lib/layoutPersistence";
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

// Layout intents: recorded, answered with a version no mocked row reaches so
// the model keeps its own tree (layoutIntents.ts compares only with a row that
// includes its intents).
const intentCalls = vi.hoisted(() => [] as { command: string; data: any }[]);
vi.mock("@/app/store/rpc-util", () => ({
    TabRpcClient: {
        rpcCall: async (command: string, data: any) => {
            intentCalls.push({ command, data });
            return { applied: true, version: Number.MAX_SAFE_INTEGER };
        },
    },
}));

vi.mock("@/app/store/global", () => {
    return {
        MOS: {
            makeORef: (_otype: string, oid: string) => oid,
            getMuxObjectAtom: (oid: string) => {
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

// SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md: `requestNodeFocus()` used to be
// a no-op, so InsertNode/FocusNode/MagnifyNodeToggle calling it here was
// inert. Now real, it reaches into `@/app/store/global`'s `atoms` (which
// this file's own minimal mock above deliberately doesn't provide — these
// tests exercise pure tree-mutation logic, not app-wide focus plumbing) via
// `getLayoutModelForStaticTab()`. Mocked out entirely rather than backfilled
// into the mock above, same reasoning as that mock's own narrow surface.
vi.mock("@/app/store/focusManager", () => ({
    focusManager: {
        requestNodeFocus: () => {},
        refocusNode: () => {},
        claimFocusOnMount: () => {},
        ensureSelectionFocused: () => {},
    },
}));

// -- Helpers ------------------------------------------------------------------

function createLayoutModel(): LayoutModel {
    const [getTab] = createSignal<Tab>({
        otype: "tab",
        oid: "tab-1",
        version: 1,
        meta: {},
        name: "Test",
        layoutstate: "layout-1",
        // Every block id these tests mount or insert-via-action.
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
    const muxObj = model.getter(model.muxObjectAtom) as LayoutState;
    const sig = layoutStateSignals.get(muxObj.oid);
    if (sig) sig._set({ ...muxObj, pendingbackendactions: actions });
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

    // SPEC_MACOS_BROWSER_PANE_KEYBOARD_FOCUS_2026_09_24 §frontend: a
    // split-placed open (`muxsh web`, which always splits next to the calling
    // terminal) dropped the action's `focused` flag, so the new pane was never
    // selected and never claimed the caret.
    it.each([
        [LayoutTreeActionType.SplitHorizontal],
        [LayoutTreeActionType.SplitVertical],
    ])("%s with focused:true selects the new block", async (actiontype) => {
        const model = createLayoutModel();
        const existing = insertBlock(model, "existing");
        expect(model.treeState.focusedNodeId).toBe(existing.id);

        setPendingActions(model, [{
            actiontype,
            actionid: `action-focus-${actiontype}`,
            blockid: "new-block",
            targetblockid: "existing",
            position: "after",
            focused: true,
            magnified: false,
            ephemeral: false,
        }]);
        await processPendingBackendActions(model);

        expect(model.treeState.focusedNodeId).toBe(findBlock(model, "new-block")!.id);
    });

    it("split with focused:false leaves the selection where it was", async () => {
        const model = createLayoutModel();
        const existing = insertBlock(model, "existing");

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.SplitHorizontal,
            actionid: "action-nofocus",
            blockid: "new-block",
            targetblockid: "existing",
            position: "after",
            focused: false,
            magnified: false,
            ephemeral: false,
        }]);
        await processPendingBackendActions(model);

        expect(model.treeState.focusedNodeId).toBe(existing.id);
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

    // SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §2.2 / §3.4: a backend
    // delete names ONE block. For a stacked pane it removes that tab only —
    // deleting the whole leaf took every sibling tab (agents included) with it.
    function stackRoot(model: LayoutModel, stack: string[], visible: string) {
        insertBlock(model, stack[0]);
        const data = model.treeState.rootNode!.data!;
        data.blockStack = [...stack];
        data.activeBlockId = visible;
        data.blockId = visible;
        model.updateTree(false);
    }

    function deleteAction(blockid: string, actionid: string) {
        return {
            actiontype: LayoutTreeActionType.DeleteNode,
            actionid,
            blockid,
            targetblockid: "",
            position: "",
            nodesize: undefined,
            focused: false,
            magnified: false,
            ephemeral: false,
        };
    }

    it("DeleteNode for a background tab of a stacked pane removes only that tab", async () => {
        const model = createLayoutModel();
        stackRoot(model, ["existing", "moved-block"], "existing");

        setPendingActions(model, [deleteAction("moved-block", "del-bg")]);
        await processPendingBackendActions(model);

        const data = model.treeState.rootNode?.data;
        expect(data, "the pane must survive").toBeTruthy();
        expect(data!.blockStack).toEqual(["existing"]);
        expect(data!.blockId).toBe("existing");
    });

    it("DeleteNode for the visible tab of a stacked pane activates its right-hand neighbour", async () => {
        const model = createLayoutModel();
        stackRoot(model, ["existing", "moved-block", "new-block"], "moved-block");

        setPendingActions(model, [deleteAction("moved-block", "del-visible")]);
        await processPendingBackendActions(model);

        const data = model.treeState.rootNode!.data!;
        expect(data.blockStack).toEqual(["existing", "new-block"]);
        expect(data.activeBlockId).toBe("new-block");
        expect(data.blockId).toBe("new-block");
    });

    it("two DeleteNode actions for sibling tabs in one batch leave the pane with its remaining tab", async () => {
        // reagent P1 on #3414: `leafs` is refreshed only once per batch, so a
        // later action in the same batch can miss it and take the tree-walk
        // fallback — which must also remove just the member, not the pane.
        const model = createLayoutModel();
        stackRoot(model, ["existing", "moved-block", "new-block"], "existing");

        setPendingActions(model, [deleteAction("moved-block", "del-a"), deleteAction("new-block", "del-b")]);
        await processPendingBackendActions(model);

        const data = model.treeState.rootNode?.data;
        expect(data, "the pane must survive").toBeTruthy();
        expect(data!.blockStack).toEqual(["existing"]);
        expect(data!.blockId).toBe("existing");
    });

    // SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.3: the backend created
    // and placed the block (CreateBlockInStack); the frontend mirrors it.
    it("StackPush adds the block as the visible tab of the pane holding the target", async () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.StackPush,
            actionid: "stackpush-1",
            blockid: "new-block",
            targetblockid: "existing",
            position: "",
            nodesize: undefined,
            focused: true,
            magnified: false,
            ephemeral: false,
        }]);
        await processPendingBackendActions(model);

        const data = model.treeState.rootNode!.data!;
        expect(data.blockStack).toEqual(["existing", "new-block"]);
        expect(data.blockId).toBe("new-block");
        expect(data.activeBlockId).toBe("new-block");
    });

    it("StackPush for a block already added locally is idempotent", async () => {
        const model = createLayoutModel();
        stackRoot(model, ["existing", "new-block"], "new-block");

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.StackPush,
            actionid: "stackpush-2",
            blockid: "new-block",
            targetblockid: "existing",
            position: "",
            nodesize: undefined,
            focused: true,
            magnified: false,
            ephemeral: false,
        }]);
        await processPendingBackendActions(model);

        expect(model.treeState.rootNode!.data!.blockStack).toEqual(["existing", "new-block"]);
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

// Another window watching this layout applies the queued `stackmove` that
// `pane.moveTab` sends. When the moved tab was its pane's only one, that
// pane has to go here too — as a move, never through onNodeDelete.
// SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §4.2.
describe("processPendingBackendActions — StackMove of a pane's only tab", () => {
    beforeEach(() => {
        layoutStateSignals.clear();
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it("moves the tab and removes the emptied pane without deleting the block", async () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");
        insertBlock(model, "moved-block");
        model.updateTree();
        const onNodeDelete = vi.fn().mockResolvedValue(undefined);
        const beforeNodeDelete = vi.fn().mockResolvedValue(true);
        model.onNodeDelete = onNodeDelete;
        model.beforeNodeDelete = beforeNodeDelete;

        setPendingActions(model, [{
            actiontype: LayoutTreeActionType.StackMove,
            actionid: "action-stackmove-last",
            blockid: "moved-block",
            targetblockid: "existing",
            position: "end",
            focused: true,
            magnified: false,
            ephemeral: false,
        }]);

        await processPendingBackendActions(model);

        const leaf = findBlock(model, "existing")!;
        expect(leaf.data!.blockStack).toEqual(["existing", "moved-block"]);
        expect(leaf.data!.activeBlockId).toBe("moved-block");
        expect(findBlock(model, "moved-block")!.id).toBe(leaf.id);
        expect(model.leafs()).toHaveLength(1);
        expect(onNodeDelete).not.toHaveBeenCalled();
        expect(beforeNodeDelete).not.toHaveBeenCalled();
    });
});

// A tab's model is disposed when the window tab leaves this window
// (installLayoutModelEviction). Its tree is stale from then on.
describe("a disposed model", () => {
    beforeEach(() => {
        layoutStateSignals.clear();
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it("ignores backend updates", () => {
        const model = createLayoutModel();
        model.dispose();
        const getter = vi.spyOn(model, "getter");
        model.onBackendUpdate();
        expect(getter).not.toHaveBeenCalled();
    });
});

// SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.3, §3.5.
describe("pane-tab actions", () => {
    beforeEach(() => {
        layoutStateSignals.clear();
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    function stackAction(actiontype: LayoutTreeActionType, actionid: string, blockid: string, targetblockid = "") {
        return { actiontype, actionid, blockid, targetblockid, focused: true, magnified: false, ephemeral: false };
    }

    it("a stackactivate switches the visible tab, and a model skips its own", async () => {
        const model = createLayoutModel();
        insertBlock(model, "existing");
        setPendingActions(model, [stackAction(LayoutTreeActionType.StackPush, "push-1", "new-block", "existing")]);
        await processPendingBackendActions(model);
        expect(findBlock(model, "existing")!.data!.blockId).toBe("new-block");

        model.processedActionIds.add("own-1");
        setPendingActions(model, [stackAction(LayoutTreeActionType.StackActivate, "own-1", "existing")]);
        await processPendingBackendActions(model);
        expect(findBlock(model, "existing")!.data!.blockId).toBe("new-block");

        setPendingActions(model, [stackAction(LayoutTreeActionType.StackActivate, "other-1", "existing")]);
        await processPendingBackendActions(model);
        const data = findBlock(model, "existing")!.data!;
        expect(data.blockId).toBe("existing");
        expect(data.activeBlockId).toBe("existing");
        expect(data.blockStack).toEqual(["existing", "new-block"]);
    });
});
