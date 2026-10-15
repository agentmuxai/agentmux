// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A layout edit leaves as an intent, not as the tree; the outbox sends in
// order; the model shows srv's stored tree when it differs.
// SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §4.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { deriveNodeId, newLayoutNode } from "@/layout/lib/layoutNode";
import { isLayoutQuiet, reconcileWithStoredLayout } from "@/layout/lib/layoutIntents";
import { sendStackCommand } from "@/layout/lib/layoutPersistence";
import {
    LayoutNode,
    LayoutTreeActionType,
    LayoutTreeInsertNodeAction,
    LayoutTreeSplitHorizontalAction,
} from "@/layout/lib/types";
import type { SignalAtom } from "@/util/util";

type Call = { command: string; data: any; resolve: (v: unknown) => void; reject: (e: unknown) => void };
const calls = vi.hoisted(() => [] as Call[]);
vi.mock("@/app/store/rpc-util", () => ({
    TabRpcClient: {
        rpcCall: (command: string, data: any) =>
            new Promise((resolve, reject) => calls.push({ command, data, resolve, reject })),
    },
}));

const rows = new Map<string, SignalAtom<LayoutState>>();
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
        setObjectValue: () => {},
    },
    getSettingsKeyAtom: () => createSignal(0.75)[0],
}));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: { requestNodeFocus: () => {}, refocusNode: () => {}, ensureSelectionFocused: () => {}, claimFocusOnMount: () => {} },
}));

function createModel(): LayoutModel {
    const [tab] = createSignal<Tab>({ otype: "tab", oid: "tab-1", version: 1, meta: {}, name: "t", layoutstate: "layout-1", blockids: [] });
    const model = new LayoutModel(tab);
    model.getBoundingRect = () => ({ top: 0, left: 0, width: 800, height: 600 });
    model.displayContainerRef.current = { getBoundingClientRect: () => ({ top: 0, left: 0, width: 800, height: 600 }) } as any;
    return model;
}

/** srv stores `rootnode` at `version` and the bridge broadcasts the row. */
function storeRow(model: LayoutModel, version: number, rootnode: LayoutNode | undefined, focused = "") {
    (rows.get("layout-1") as any)._set({ otype: "layout", oid: "layout-1", version, meta: {}, rootnode, focusednodeid: focused });
    model.onBackendUpdate();
}

const settle = async () => {
    for (let i = 0; i < 20; i++) await Promise.resolve();
};

function intentsSent() {
    return calls.filter((c) => c.command === "layout.applyIntent");
}

function insert(model: LayoutModel, blockId: string): LayoutNode {
    const node = newLayoutNode(undefined, undefined, undefined, { blockId });
    model.treeReducer({ type: LayoutTreeActionType.InsertNode, node, magnified: false, focused: true } as LayoutTreeInsertNodeAction);
    return node;
}

describe("layout intents", () => {
    beforeEach(() => {
        rows.clear();
        calls.length = 0;
    });
    afterEach(() => vi.useRealTimers());

    it("an edit is sent as an intent, never as the tree", async () => {
        const model = createModel();
        const push = vi.fn(async () => null);
        (model.muxObjectAtom as any)._push = push;
        const a = insert(model, "a");
        await settle();
        expect(intentsSent()).toHaveLength(1);
        const sent = intentsSent()[0].data;
        expect(sent.tab_id).toBe("tab-1");
        expect(sent.intent).toEqual({ type: "insert", node: JSON.parse(JSON.stringify(a)), magnified: false, focused: true });
        expect(sent.focused_node_id).toBe(a.id);
        expect(push).not.toHaveBeenCalled();
    });

    it("the intent carries the new node as it was before the edit, and the branches are named from the action id", async () => {
        const model = createModel();
        const a = insert(model, "a");
        intentsSent()[0].resolve({ applied: true, version: 2 });
        await settle();
        const b = newLayoutNode(undefined, undefined, undefined, { blockId: "b" });
        model.treeReducer({
            type: LayoutTreeActionType.SplitHorizontal,
            targetNodeId: a.id,
            newNode: b,
            position: "after",
            sizeFraction: 0.5,
            focused: true,
        } as LayoutTreeSplitHorizontalAction);
        await settle();
        const sent = intentsSent()[1].data;
        expect(sent.intent.type).toBe("splithorizontal");
        expect(sent.intent.newNode.size).toBe(10); // the local edit carved it to 5 afterwards
        expect(model.treeState.rootNode!.id).toBe(deriveNodeId(sent.action_id, 1));
    });

    it("intents leave one at a time, in order", async () => {
        const model = createModel();
        insert(model, "a");
        insert(model, "b");
        await settle();
        expect(intentsSent()).toHaveLength(1);
        intentsSent()[0].resolve({ applied: true, version: 2 });
        await settle();
        expect(intentsSent()).toHaveLength(2);
        expect(intentsSent()[1].data.intent.node.data.blockId).toBe("b");
    });

    it("a whole-tree push waits for the intents ahead of it and carries the ones behind it", async () => {
        vi.useFakeTimers();
        const model = createModel();
        const pushed: LayoutState[] = [];
        (model.muxObjectAtom as any)._push = vi.fn(async (v: LayoutState) => void pushed.push(JSON.parse(JSON.stringify(v))));
        insert(model, "a");
        model.persistToBackend();
        await vi.advanceTimersByTimeAsync(150);
        insert(model, "b"); // queued behind the push
        await settle();
        expect(pushed).toHaveLength(0);
        intentsSent()[0].resolve({ applied: true, version: 2 });
        await vi.advanceTimersByTimeAsync(0);
        await settle();
        expect(pushed).toHaveLength(1);
        expect(pushed[0].rootnode.children?.length ?? 0).toBe(2); // b is in it
        expect(intentsSent()).toHaveLength(1); // so b's intent was dropped
        expect(isLayoutQuiet(model)).toBe(true);
    });

    it("a resend after a failed send keeps the action id", async () => {
        vi.useFakeTimers();
        const model = createModel();
        insert(model, "a");
        await settle();
        intentsSent()[0].reject(new Error("socket closed"));
        await vi.advanceTimersByTimeAsync(200);
        expect(intentsSent()).toHaveLength(2);
        expect(intentsSent()[1].data.action_id).toBe(intentsSent()[0].data.action_id);
    });

    describe("showing srv's stored tree", () => {
        it("replaces the local tree when srv stored something else", async () => {
            const model = createModel();
            const a = insert(model, "a");
            intentsSent()[0].resolve({ applied: false, error: "refused", version: 3 });
            await settle();
            const stored = { id: "srv-root", flexDirection: "row", size: 10, data: { blockId: "z" } } as LayoutNode;
            storeRow(model, 3, stored);
            expect(model.treeState.rootNode).toEqual(stored);
            expect(model.treeState.rootNode).not.toBe(stored); // a copy: local edits must not reach the row cache
            expect(model.treeState.focusedNodeId).not.toBe(a.id);
        });

        it("waits for the row that includes its intents", async () => {
            const model = createModel();
            const a = insert(model, "a");
            intentsSent()[0].resolve({ applied: true, version: 5 });
            await settle();
            storeRow(model, 4, undefined); // older than the intent's answer
            expect(model.treeState.rootNode?.id).toBe(a.id);
            storeRow(model, 5, model.treeState.rootNode && JSON.parse(JSON.stringify(model.treeState.rootNode)));
            expect(model.treeState.rootNode?.id).toBe(a.id);
        });

        it("does nothing while an intent is on its way", async () => {
            const model = createModel();
            const a = insert(model, "a");
            await settle();
            storeRow(model, 9, undefined);
            expect(model.treeState.rootNode?.id).toBe(a.id);
            reconcileWithStoredLayout(model);
            expect(model.treeState.rootNode?.id).toBe(a.id);
        });

        it("leaves the local tree alone when srv's normalizes to it", async () => {
            const model = createModel();
            insert(model, "a");
            insert(model, "b");
            intentsSent()[0].resolve({ applied: true, version: 2 });
            await settle();
            intentsSent()[1].resolve({ applied: true, version: 3 });
            await settle();
            const local = model.treeState.rootNode!;
            const unbalanced = JSON.parse(JSON.stringify(local));
            for (const child of unbalanced.children) child.flexDirection = unbalanced.flexDirection;
            storeRow(model, 3, unbalanced);
            expect(model.treeState.rootNode).toBe(local);
        });

        it("keeps the frontend's focus while its pane exists", async () => {
            const model = createModel();
            const a = insert(model, "a");
            const b = insert(model, "b");
            intentsSent()[0].resolve({ applied: true, version: 2 });
            await settle();
            intentsSent()[1].resolve({ applied: true, version: 3 });
            await settle();
            const stored = JSON.parse(JSON.stringify(model.treeState.rootNode));
            stored.size = 7; // srv's differs only in size
            model.treeState.focusedNodeId = a.id;
            storeRow(model, 3, stored, b.id);
            expect(model.treeState.rootNode!.size).toBe(7);
            expect(model.treeState.focusedNodeId).toBe(a.id);
        });
    });
});

describe("backend edits srv already applied", () => {
    beforeEach(() => {
        rows.clear();
        calls.length = 0;
    });

    /** srv placed `blockId` (as `place_block` does) and queued the matching action. */
    function queueInsert(model: LayoutModel, version: number, stored: LayoutNode | undefined, action: Partial<LayoutActionData>) {
        const row = rows.get("layout-1") as any;
        row._set({
            otype: "layout",
            oid: "layout-1",
            version,
            meta: {},
            rootnode: stored,
            pendingbackendactions: [{ actiontype: "insert", focused: true, magnified: false, ephemeral: false, ...action }],
        });
        model.onBackendUpdate();
    }

    it("a queued insert is mirrored with srv's node id, and nothing is sent back", async () => {
        const model = createModel();
        const a = insert(model, "a");
        intentsSent()[0].resolve({ applied: true, version: 2 });
        await settle();
        const push = vi.fn(async () => null);
        (model.muxObjectAtom as any)._push = push;
        // srv's tree after its own insert: a and the new leaf in a row.
        const actionId = "a0000009-0000-4000-8000-000000000009";
        const stored = {
            id: deriveNodeId(actionId, 1),
            flexDirection: "row",
            size: 10,
            children: [
                { ...JSON.parse(JSON.stringify(a)), flexDirection: "column" },
                { id: "srv-leaf", flexDirection: "column", size: 10, data: { blockId: "b" } },
            ],
        } as LayoutNode;
        queueInsert(model, 3, stored, { actionid: actionId, blockid: "b", nodeid: "srv-leaf" });
        await settle();
        expect(JSON.stringify(model.treeState.rootNode)).toContain("srv-leaf");
        expect(model.treeState.rootNode!.id).toBe(deriveNodeId(actionId, 1));
        expect(intentsSent()).toHaveLength(1); // only the first insert
        await new Promise((r) => setTimeout(r, 150));
        expect(push).not.toHaveBeenCalled();
    });

    it("a queued placement already in the tree is skipped", async () => {
        const model = createModel();
        insert(model, "a");
        intentsSent()[0].resolve({ applied: true, version: 2 });
        await settle();
        const before = JSON.stringify(model.treeState.rootNode);
        queueInsert(model, 2, model.treeState.rootNode, { actionid: "x1", blockid: "a", nodeid: "other" });
        await settle();
        expect(JSON.stringify(model.treeState.rootNode)).toBe(before);
    });

    it("a model loading a row takes its queued actions as seen, not to replay", () => {
        const [get, set] = createSignal<LayoutState>({
            otype: "layout",
            oid: "layout-1",
            version: 4,
            meta: {},
            rootnode: { id: "r", flexDirection: "row", size: 10, data: { blockId: "b" } },
            pendingbackendactions: [
                { actiontype: "insert", actionid: "q1", blockid: "b", focused: true, magnified: false, ephemeral: false },
            ],
        } as LayoutState);
        const atom = () => get();
        (atom as any)._set = set;
        rows.set("layout-1", atom as unknown as SignalAtom<LayoutState>);
        const model = createModel();
        expect(model.processedActionIds.has("q1")).toBe(true);
        expect(model.treeState.rootNode!.id).toBe("r");
        expect(calls).toHaveLength(0);
    });

    it("a pane-tab command waits for the intents queued before it", async () => {
        const model = createModel();
        insert(model, "a");
        await settle();
        let sentCommand = false;
        const done = sendStackCommand(model, "pane.activateTab", async () => {
            sentCommand = true;
            return { layout_revision: 0 };
        });
        await settle();
        expect(sentCommand).toBe(false);
        intentsSent()[0].resolve({ applied: true, version: 2 });
        await done;
        expect(sentCommand).toBe(true);
    });

    it("closing the ephemeral pane sends only the view", async () => {
        const model = createModel();
        insert(model, "a");
        intentsSent()[0].resolve({ applied: true, version: 2 });
        await settle();
        model.newEphemeralNode("eph");
        const eph = model.ephemeralNode();
        await model.closeNode(eph.id);
        await settle();
        expect(intentsSent().map((c) => c.data.intent.type)).toEqual(["insert", "view"]);
    });
});
