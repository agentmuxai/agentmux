// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the in-pane-tabs block-stack mechanism (layoutStack.ts) and
 * the tile renderer's stack-aware remount key (activeKeyFor).
 * Spec: docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.3.
 */

import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { newLayoutNode } from "@/layout/lib/layoutNode";
import { activeKeyFor, getNodeByBlockId } from "@/layout/lib/layoutNodeModels";
import { closeBlockInStack, pushBlockOntoStack, setActiveBlockInStack } from "@/layout/lib/layoutStack";
import { LayoutNodeAdditionalProps, LayoutTreeActionType, LayoutTreeInsertNodeAction } from "@/layout/lib/types";
import type { SignalAtom } from "@/util/util";

// Same mock harness as layoutModel.test.ts.
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
    model.getBoundingRect = () => ({ top: 0, left: 0, width: 800, height: 600 });
    model.displayContainerRef.current = {
        getBoundingClientRect: () => ({ top: 0, left: 0, width: 800, height: 600 }),
    } as any;
    return model;
}

/** Insert a single leaf as the tab's root and return its node id. */
function insertRootBlock(model: LayoutModel, blockId: string): string {
    const node = newLayoutNode(undefined, undefined, undefined, { blockId });
    model.treeReducer({
        type: LayoutTreeActionType.InsertNode,
        node,
        magnified: false,
        focused: true,
    } as LayoutTreeInsertNodeAction);
    return model.treeState.rootNode!.id;
}

describe("layoutStack", () => {
    beforeEach(() => {
        layoutStateSignals.clear();
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    describe("pushBlockOntoStack", () => {
        it("turns a non-stacked leaf into a 2-member stack, active = the new block", () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");

            pushBlockOntoStack(model, nodeId, "b2");

            const data = model.treeState.rootNode!.data!;
            expect(data.blockStack).toEqual(["b1", "b2"]);
            expect(data.activeBlockId).toBe("b2");
            expect(data.blockId).toBe("b2"); // legacy field stays in sync
        });

        it("does NOT evict the node's cached NodeModel — the leaf stays mounted across a switch", () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            model.getNodeModel(model.treeState.rootNode!); // populate the cache
            expect(model.nodeModels.has(nodeId)).toBe(true);

            pushBlockOntoStack(model, nodeId, "b2");

            expect(model.nodeModels.has(nodeId)).toBe(true);
        });

        // SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md: eviction here
        // used to be required because the OUTER leaf-level key
        // (tilelayout-shared.tsx's DisplayNodesWrapper) used to include
        // activeBlockId, forcing a full leaf remount on every switch. That
        // key is now `node.id` alone — pane-leaf-chrome.tsx owns a
        // narrower, INNER remount instead (scoped to just the active
        // block's own <Block> instance) — so the leaf's NodeModel is no
        // longer torn down on a switch; a still-mounted leaf component
        // depends on it staying live. This proves that by DIFFERENTIATING a
        // live memo from a frozen one: a disposed memo's return value
        // freezes at whatever it was computed as right before disposal and
        // does not react to a LATER dependency change, whereas a live one
        // keeps updating. Falsifiable: reintroducing a `disposeNodeModel`
        // call in `pushBlockOntoStack` makes this test fail, because the
        // assertion below would instead observe the frozen, stale value.
        it("keeps the NodeModel's reactive root alive across the switch, not just its map entry", () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            const node = model.treeState.rootNode!;

            const nodeModel = model.getNodeModel(node);
            expect(model.nodeModelDisposers.has(nodeId)).toBe(true);
            // insertRootBlock inserts with `focused: true` — the lone leaf
            // starts out focused by default.
            expect(nodeModel.isFocused()).toBe(true);

            pushBlockOntoStack(model, nodeId, "b2"); // must NOT dispose
            expect(model.nodeModelDisposers.has(nodeId)).toBe(true); // disposer still tracked — not consumed

            // Un-focus — the identical treeState mutation layoutFocus.ts's
            // own validateFocusedNode uses internally
            // (model.treeState.focusedNodeId = id;
            // model.setter(model.localTreeStateAtom, {...})), not the
            // public focusNode() action, since there's no second node here
            // to focus instead and that action's existence-check isn't the
            // concern under test. A disposed isFocused memo would stay
            // frozen at true; a live one reacts to false.
            model.treeState.focusedNodeId = undefined;
            model.setter(model.localTreeStateAtom, { ...model.treeState });
            expect(nodeModel.isFocused()).toBe(false); // live — proves NOT disposed
        });

        // Same live-vs-frozen differentiation as the test above, targeting
        // additionalProps specifically — this field is built by its own
        // separate memo inside getNodeModel (see that function's own
        // comment), so it needs its own proof that it isn't disposed either.
        it("keeps additionalProps' own memo alive too, not just the fields built inside the cache-miss block", () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            const node = model.treeState.rootNode!;

            const nodeModel = model.getNodeModel(node);
            // The lone leaf already has real geometry (from
            // createLayoutModel's mocked 800x600 bounding rect) — capture
            // it rather than assume undefined.
            const initialAddlProps = nodeModel.additionalProps();
            expect(initialAddlProps).toBeDefined();

            pushBlockOntoStack(model, nodeId, "b2"); // must NOT dispose

            // Set additionalProps for this node to a DIFFERENT value after
            // the switch. A disposed memo would stay frozen at the initial
            // value; a live one reads the new value.
            model.setter(model.additionalProps, {
                [nodeId]: { treeKey: "test" } as LayoutNodeAdditionalProps,
            });
            expect(nodeModel.additionalProps()).toEqual({ treeKey: "test" }); // live — proves NOT disposed
        });

        it("appending an already-present blockId re-activates it instead of duplicating the stack entry", () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            pushBlockOntoStack(model, nodeId, "b2");
            pushBlockOntoStack(model, nodeId, "b3");

            pushBlockOntoStack(model, nodeId, "b1");

            const data = model.treeState.rootNode!.data!;
            expect(data.blockStack).toEqual(["b1", "b2", "b3"]);
            expect(data.activeBlockId).toBe("b1");
        });

        it("is a no-op (no eviction, no persist) when the blockId is already active", () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            pushBlockOntoStack(model, nodeId, "b2");
            model.getNodeModel(model.treeState.rootNode!);

            pushBlockOntoStack(model, nodeId, "b2");

            expect(model.nodeModels.has(nodeId)).toBe(true); // untouched — no eviction fired
        });
    });

    describe("setActiveBlockInStack", () => {
        it("switches the active member among existing stack members", () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            pushBlockOntoStack(model, nodeId, "b2");
            pushBlockOntoStack(model, nodeId, "b3"); // active is now b3

            setActiveBlockInStack(model, nodeId, "b1");

            const data = model.treeState.rootNode!.data!;
            expect(data.activeBlockId).toBe("b1");
            expect(data.blockId).toBe("b1");
            expect(data.blockStack).toEqual(["b1", "b2", "b3"]); // membership/order untouched
        });

        it("is a no-op for a blockId that is not a stack member", () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            pushBlockOntoStack(model, nodeId, "b2");
            const before = { ...model.treeState.rootNode!.data! };

            setActiveBlockInStack(model, nodeId, "not-a-member");

            expect(model.treeState.rootNode!.data).toEqual(before);
        });

        it("does NOT evict the cached NodeModel, on a real switch or on a no-op re-activation", () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            pushBlockOntoStack(model, nodeId, "b2"); // active = b2
            model.getNodeModel(model.treeState.rootNode!);

            setActiveBlockInStack(model, nodeId, "b2"); // already active — no-op
            expect(model.nodeModels.has(nodeId)).toBe(true);

            setActiveBlockInStack(model, nodeId, "b1"); // real switch
            expect(model.nodeModels.has(nodeId)).toBe(true);
        });
    });

    describe("closeBlockInStack", () => {
        it("delegates to closeNode (removes the whole leaf) when the leaf has no stack at all", async () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            const onNodeDelete = vi.fn().mockResolvedValue(undefined);
            model.onNodeDelete = onNodeDelete;

            await closeBlockInStack(model, nodeId, "b1");

            expect(model.treeState.rootNode).toBeUndefined();
            expect(onNodeDelete).toHaveBeenCalledWith(expect.objectContaining({ blockId: "b1" }));
        });

        it("is a no-op — does NOT close the pane — when blockId doesn't match a non-stacked leaf's block", async () => {
            // Review finding: the stack.length<=1 branch used to delegate to
            // closeNode(nodeId) unconditionally, without checking blockId
            // actually belongs to this leaf — a stale/wrong id from a caller
            // could silently close a pane it doesn't own.
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            const onNodeDelete = vi.fn().mockResolvedValue(undefined);
            model.onNodeDelete = onNodeDelete;

            await closeBlockInStack(model, nodeId, "wrong-block-id");

            expect(model.treeState.rootNode).toBeDefined(); // pane survives
            expect(model.treeState.rootNode!.data!.blockId).toBe("b1");
            expect(onNodeDelete).not.toHaveBeenCalled();
        });

        it("delegates to closeNode when closing the last remaining stack member", async () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            pushBlockOntoStack(model, nodeId, "b2");
            model.onNodeDelete = vi.fn().mockResolvedValue(undefined);

            // Pop b2 back out via the normal (>1-member) path so the leaf is
            // back down to a genuine 1-member stack.
            await closeBlockInStack(model, nodeId, "b2");
            expect(model.treeState.rootNode).toBeDefined(); // leaf survived

            const onNodeDelete = vi.fn().mockResolvedValue(undefined);
            model.onNodeDelete = onNodeDelete;
            await closeBlockInStack(model, nodeId, "b1");

            expect(model.treeState.rootNode).toBeUndefined(); // whole pane closed
        });

        it("pops a non-active member without touching the tree or the active block", async () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            pushBlockOntoStack(model, nodeId, "b2");
            pushBlockOntoStack(model, nodeId, "b3"); // stack: [b1,b2,b3], active b3
            const onNodeDelete = vi.fn().mockResolvedValue(undefined);
            model.onNodeDelete = onNodeDelete;

            await closeBlockInStack(model, nodeId, "b1");

            const data = model.treeState.rootNode!.data!;
            expect(data.blockStack).toEqual(["b2", "b3"]);
            expect(data.activeBlockId).toBe("b3"); // untouched — b1 wasn't active
            expect(onNodeDelete).toHaveBeenCalledWith(expect.objectContaining({ blockId: "b1" }));
        });

        // codex P1 on #3091: closing a BACKGROUND member leaves activeBlockId
        // untouched, so activeKeyFor's key doesn't change and the leaf's
        // DisplayNode component never remounts — it's STILL holding its
        // existing NodeModel reference from its last real mount. The first
        // version of this PR's leak fix disposed unconditionally here
        // anyway, which doesn't just leak less — it actively FREEZES that
        // still-in-use component's isFocused/isMagnified/innerRect/etc,
        // since nothing will ever remount it to pick up a fresh NodeModel.
        // Same frozen-vs-live differentiation as the eviction tests above,
        // proving the opposite property this time: the memo must stay LIVE.
        it("does NOT dispose the NodeModel when closing a background member — nothing remounts to replace it", () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            pushBlockOntoStack(model, nodeId, "b2"); // stack: [b1,b2], active b2
            const node = model.treeState.rootNode!;

            const stillMountedNodeModel = model.getNodeModel(node);
            expect(model.nodeModelDisposers.has(nodeId)).toBe(true);

            void closeBlockInStack(model, nodeId, "b1"); // b1 is NOT active — background close

            // Disposer must still be there — nothing should have consumed it.
            expect(model.nodeModelDisposers.has(nodeId)).toBe(true);

            // Focus this node — a LIVE memo reacts; a wrongly-disposed one
            // would stay frozen at its pre-close value (false, since this
            // node was never explicitly focused above).
            model.treeState.focusedNodeId = nodeId;
            model.setter(model.localTreeStateAtom, { ...model.treeState });
            expect(stillMountedNodeModel.isFocused()).toBe(true); // live — proves NOT disposed
        });

        it("does NOT dispose the NodeModel when closing the active member either — the leaf still doesn't remount", () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            pushBlockOntoStack(model, nodeId, "b2"); // stack: [b1,b2], active b2
            const node = model.treeState.rootNode!;

            const nodeModel = model.getNodeModel(node);
            expect(model.nodeModelDisposers.has(nodeId)).toBe(true);

            void closeBlockInStack(model, nodeId, "b2"); // b2 IS active — activeBlockId changes, but the leaf's own key doesn't

            expect(model.nodeModelDisposers.has(nodeId)).toBe(true); // disposer still tracked — not consumed

            // Live-vs-frozen: focus the node after the close — a disposed
            // memo would stay frozen at false (never explicitly focused
            // above); a live one reacts.
            model.treeState.focusedNodeId = nodeId;
            model.setter(model.localTreeStateAtom, { ...model.treeState });
            expect(nodeModel.isFocused()).toBe(true); // live — proves NOT disposed
        });

        it("closing the ACTIVE member picks its right neighbor", async () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            pushBlockOntoStack(model, nodeId, "b2");
            pushBlockOntoStack(model, nodeId, "b3"); // stack: [b1,b2,b3], active b3
            setActiveBlockInStack(model, nodeId, "b2"); // active b2, has a right neighbor (b3)
            model.onNodeDelete = vi.fn().mockResolvedValue(undefined);

            await closeBlockInStack(model, nodeId, "b2");

            const data = model.treeState.rootNode!.data!;
            expect(data.blockStack).toEqual(["b1", "b3"]);
            expect(data.activeBlockId).toBe("b3");
        });

        it("closing the rightmost active member falls back to the new last member", async () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            pushBlockOntoStack(model, nodeId, "b2");
            pushBlockOntoStack(model, nodeId, "b3"); // active b3, rightmost
            model.onNodeDelete = vi.fn().mockResolvedValue(undefined);

            await closeBlockInStack(model, nodeId, "b3");

            const data = model.treeState.rootNode!.data!;
            expect(data.blockStack).toEqual(["b1", "b2"]);
            expect(data.activeBlockId).toBe("b2");
        });

        it("is a no-op for a blockId that is not a stack member", async () => {
            const model = createLayoutModel();
            const nodeId = insertRootBlock(model, "b1");
            pushBlockOntoStack(model, nodeId, "b2");
            const before = { ...model.treeState.rootNode!.data! };
            const onNodeDelete = vi.fn();
            model.onNodeDelete = onNodeDelete;

            await closeBlockInStack(model, nodeId, "not-a-member");

            expect(model.treeState.rootNode!.data).toEqual(before);
            expect(onNodeDelete).not.toHaveBeenCalled();
        });
    });
});

describe("getNodeByBlockId (stack-aware)", () => {
    beforeEach(() => {
        layoutStateSignals.clear();
        vi.useFakeTimers();
    });
    afterEach(() => vi.useRealTimers());

    it("finds a leaf by a dormant (non-active) stack member, not just the active blockId", () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        pushBlockOntoStack(model, nodeId, "b2"); // active = b2, b1 now dormant
        model.updateTree();

        const found = getNodeByBlockId(model, "b1");
        expect(found?.id).toBe(nodeId);
    });
});

describe("activeKeyFor", () => {
    it("keys a non-stacked leaf on its bare node id — zero behavior change for every existing pane", () => {
        const node = newLayoutNode(undefined, undefined, undefined, { blockId: "b1" });
        expect(activeKeyFor(node)).toBe(node.id);
    });

    it("keys a stacked leaf on nodeId + activeBlockId", () => {
        const node = newLayoutNode(undefined, undefined, undefined, {
            blockId: "b2",
            blockStack: ["b1", "b2"],
            activeBlockId: "b2",
        });
        expect(activeKeyFor(node)).toBe(`${node.id}:b2`);
    });

    // Still required today (ReAgent/Codex P0/P1 on PR #3132): TabContent/
    // Block render nodeModel.blockId as a plain frozen field with no
    // remount boundary of their own, so this key changing on a switch is
    // CURRENTLY the only mechanism that updates the displayed block. See
    // this file's own header comment and layoutStack.ts's for the full
    // rationale, and do not drop activeBlockId from this key until a
    // narrower inner remount boundary (pane-leaf-chrome.tsx) lands in the
    // same change.
    it("switching the active member changes the key — this is what drives the remount", () => {
        const node = newLayoutNode(undefined, undefined, undefined, {
            blockId: "b1",
            blockStack: ["b1", "b2"],
            activeBlockId: "b1",
        });
        const keyBefore = activeKeyFor(node);
        node.data!.activeBlockId = "b2";
        node.data!.blockId = "b2";
        const keyAfter = activeKeyFor(node);
        expect(keyBefore).not.toBe(keyAfter);
    });
});

describe("NodeModel.activeBlockId", () => {
    beforeEach(() => {
        layoutStateSignals.clear();
        vi.useFakeTimers();
    });
    afterEach(() => vi.useRealTimers());

    // Consumed by pane-leaf-chrome.tsx's inner <Key> and AgentPaneChrome
    // (agent-view.tsx) to track the active stack member without the leaf
    // itself remounting. Proves the memo re-derives from live tree state on
    // every read rather than freezing at construction time, the same
    // pattern already proven safe in agent-view.tsx's own (now-superseded)
    // activeBlockId memo. Mutates tree state directly (not via
    // pushBlockOntoStack) so this test doesn't depend on whether that
    // mutator disposes the NodeModel — that's a separate invariant covered
    // by the describe blocks above (it doesn't, as of this file).
    it("tracks the currently active stack member from live tree state", () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        const node = model.treeState.rootNode!;

        const nodeModel = model.getNodeModel(node);
        expect(nodeModel.activeBlockId!()).toBe("b1");

        node.data!.activeBlockId = "b2";
        node.data!.blockId = "b2";
        model.setter(model.localTreeStateAtom, { ...model.treeState });

        expect(nodeModel.activeBlockId!()).toBe("b2");
    });

    it("the SAME NodeModel instance picks up the new active member after a switch (no eviction)", () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        const nodeModel = model.getNodeModel(model.treeState.rootNode!);

        pushBlockOntoStack(model, nodeId, "b2");

        expect(model.getNodeModel(model.treeState.rootNode!)).toBe(nodeModel); // same instance — not evicted
        expect(nodeModel.activeBlockId!()).toBe("b2");
    });
});

describe("NodeModel.hasEverBeenMultiMember", () => {
    beforeEach(() => {
        layoutStateSignals.clear();
        vi.useFakeTimers();
    });
    afterEach(() => vi.useRealTimers());

    // Drives AgentViewModel.noHeader (agent-model.ts) — must be MONOTONIC,
    // never resetting to false once true, or a hoisted header would tear
    // down and rebuild the instant a stack shrank back to 1 member. See
    // this field's own doc comment (types.ts).
    it("is false for a non-stacked leaf and flips permanently true once the stack ever exceeds 1 member", () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        const nodeModel = model.getNodeModel(model.treeState.rootNode!);
        expect(nodeModel.hasEverBeenMultiMember!()).toBe(false);

        pushBlockOntoStack(model, nodeId, "b2"); // stack: [b1,b2]
        expect(nodeModel.hasEverBeenMultiMember!()).toBe(true);

        // Shrink back to 1 member — must NOT reset.
        void closeBlockInStack(model, nodeId, "b2");
        expect(nodeModel.hasEverBeenMultiMember!()).toBe(true);
    });
});

describe("NodeModel.activeViewModel / setActiveViewModel", () => {
    beforeEach(() => {
        layoutStateSignals.clear();
        vi.useFakeTimers();
    });
    afterEach(() => vi.useRealTimers());

    // block.tsx pushes into this on mount/unmount (see that file's own
    // comments) — this suite exercises just the NodeModel-side contract in
    // isolation: a plain owner-checked signal pair, mirroring
    // unregisterBlockComponentModel's owner-check pattern
    // (block-component-registry.ts) so a stale/delayed cleanup from an
    // older "owner" can never clobber a newer one's live registration.
    it("starts null, is set by its owner, and reads back the set value", () => {
        const model = createLayoutModel();
        insertRootBlock(model, "b1");
        const nodeModel = model.getNodeModel(model.treeState.rootNode!);
        expect(nodeModel.activeViewModel!()).toBeNull();

        const owner = {};
        const vm = { viewType: "agent" } as unknown as ViewModel;
        nodeModel.setActiveViewModel!(vm, owner);

        expect(nodeModel.activeViewModel!()).toBe(vm);
    });

    it("a non-owning caller cannot clear a live registration", () => {
        const model = createLayoutModel();
        insertRootBlock(model, "b1");
        const nodeModel = model.getNodeModel(model.treeState.rootNode!);

        const owner = {};
        const vm = { viewType: "agent" } as unknown as ViewModel;
        nodeModel.setActiveViewModel!(vm, owner);

        nodeModel.setActiveViewModel!(null, {}); // different owner — stale/delayed cleanup
        expect(nodeModel.activeViewModel!()).toBe(vm); // untouched
    });

    it("the owning caller can clear its own registration", () => {
        const model = createLayoutModel();
        insertRootBlock(model, "b1");
        const nodeModel = model.getNodeModel(model.treeState.rootNode!);

        const owner = {};
        const vm = { viewType: "agent" } as unknown as ViewModel;
        nodeModel.setActiveViewModel!(vm, owner);

        nodeModel.setActiveViewModel!(null, owner);
        expect(nodeModel.activeViewModel!()).toBeNull();
    });
});

describe("LayoutModel.dispose()", () => {
    beforeEach(() => {
        layoutStateSignals.clear();
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    // reagent P1 on #3091, round 2: `dispose()` (ordinary tab close, via
    // `deleteLayoutModelForTab`) used to only call `this._disposeRoot()` —
    // which tears down `_modelOwner` — and never iterated
    // `nodeModelDisposers`. `createRoot` (what `getNodeModel` nests every
    // NodeModel's memos in, for the per-node disposal boundary the other
    // tests in this file exercise) is DELIBERATELY DETACHED from whatever
    // owner is ambient when it's called — confirmed empirically with a
    // standalone scratch test before trusting this, since this PR had
    // already gotten SolidJS ownership semantics wrong twice. So disposing
    // only `_modelOwner` never reached any NodeModel's nested root: every
    // leaf whose NodeModel was never explicitly evicted before the whole
    // tab closed (the common case) leaked its ~10-memo root forever — the
    // same class of bug this PR fixes, just on the far more common "close a
    // tab" path instead of an in-pane switch. Falsifiable: removing the
    // `for (const nodeid of [...this.nodeModelDisposers.keys()])` loop in
    // `LayoutModel.dispose()` (layoutModel.ts) makes this test fail, since
    // the frozen assertion below would instead observe the live, updated
    // value, and the map-emptiness assertions would fail too.
    it("disposes every cached NodeModel's reactive root on tab close, not just the model's own root", () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        const node = model.treeState.rootNode!;

        const nodeModel = model.getNodeModel(node);
        expect(model.nodeModelDisposers.has(nodeId)).toBe(true);
        // insertRootBlock inserts with `focused: true` — the lone leaf
        // starts out focused by default.
        expect(nodeModel.isFocused()).toBe(true);

        model.dispose();

        // Map-level cleanup: nothing left cached or tracked for disposal.
        expect(model.nodeModelDisposers.size).toBe(0);
        expect(model.nodeModels.size).toBe(0);

        // Frozen-vs-live: mutate the same underlying signal `isFocused`
        // depends on (`localTreeStateAtom`, via the mocked WOS layer — this
        // still works after dispose() since the mock signal itself isn't
        // torn down, only the model's own reactive graph). A live memo
        // would now read false; a disposed one stays frozen at whatever it
        // was computed as right before disposal.
        model.treeState.focusedNodeId = undefined;
        model.setter(model.localTreeStateAtom, { ...model.treeState });
        expect(nodeModel.isFocused()).toBe(true); // frozen — proves disposal, not just eviction
    });
});
