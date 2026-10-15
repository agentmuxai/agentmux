// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The frontend predicts every layout edit with its own reducer, and srv stores
// its own result for the same intent (layoutIntents.ts). The two must agree:
// this replays srv's random intent sequences, generated with srv's results by
// crates/srv/src/backend/layout/intent/tests.rs, through the frontend's tree
// functions and balance pass. SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §8.

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { LayoutIntent } from "@/types/rpc/LayoutIntent";
import { applyLayoutAction, sameLayoutTree } from "../lib/layoutIntentActions";
import { balanceNode, findNode, withDerivedNodeIds } from "../lib/layoutNode";
import { LayoutNode, LayoutTreeAction, LayoutTreeActionType, LayoutTreeState } from "../lib/types";

type Vector = {
    seed: number;
    initial: LayoutNode | null;
    steps: { actionId: string; intent: LayoutIntent; applied: boolean }[];
    final: LayoutNode | null;
};

const file = JSON.parse(readFileSync(new URL("./fixtures/layout-intent-vectors.json", import.meta.url), "utf8"));
const vectors: Vector[] = file.vectors;

const copy = <T>(v: T): T => (v === undefined ? v : JSON.parse(JSON.stringify(v)));

/** The frontend action an intent came from (the inverse of `intentForAction`). */
function actionFor(intent: LayoutIntent, root: LayoutNode | undefined): LayoutTreeAction | null {
    switch (intent.type) {
        case "insert":
            return { type: LayoutTreeActionType.InsertNode, node: copy(intent.node), magnified: intent.magnified, focused: intent.focused } as LayoutTreeAction;
        case "insertatindex":
            return {
                type: LayoutTreeActionType.InsertNodeAtIndex,
                node: copy(intent.node),
                indexArr: [...intent.indexArr],
                magnified: intent.magnified,
                focused: intent.focused,
            } as LayoutTreeAction;
        case "delete":
            return { type: LayoutTreeActionType.DeleteNode, nodeId: intent.nodeId } as LayoutTreeAction;
        case "move":
            return {
                type: LayoutTreeActionType.Move,
                node: (root && findNode(root, intent.nodeId)) ?? { id: intent.nodeId },
                parentId: intent.parentId,
                index: intent.index,
                insertAtRoot: intent.insertAtRoot,
            } as LayoutTreeAction;
        case "swap":
            return { type: LayoutTreeActionType.Swap, node1Id: intent.node1Id, node2Id: intent.node2Id } as LayoutTreeAction;
        case "resize":
            return { type: LayoutTreeActionType.ResizeNode, resizeOperations: intent.resizeOperations } as LayoutTreeAction;
        case "replace":
            return {
                type: LayoutTreeActionType.ReplaceNode,
                targetNodeId: intent.targetNodeId,
                newNode: copy(intent.newNode),
                focused: intent.focused,
            } as LayoutTreeAction;
        case "splithorizontal":
        case "splitvertical":
            return {
                type: intent.type === "splithorizontal" ? LayoutTreeActionType.SplitHorizontal : LayoutTreeActionType.SplitVertical,
                targetNodeId: intent.targetNodeId,
                newNode: copy(intent.newNode),
                position: intent.position,
                sizeFraction: intent.sizeFraction,
                focused: intent.focused,
            } as LayoutTreeAction;
        case "minimize":
            return { type: LayoutTreeActionType.Minimize, nodeId: intent.nodeId } as LayoutTreeAction;
        case "restore":
            return { type: LayoutTreeActionType.Restore, nodeId: intent.nodeId } as LayoutTreeAction;
        case "clear":
            return { type: LayoutTreeActionType.ClearTree } as LayoutTreeAction;
        case "view":
            return null;
    }
}

/** One step as `LayoutModel.treeReducer` + `updateTree` run it. */
function step(state: LayoutTreeState, actionId: string, intent: LayoutIntent): void {
    const action = actionFor(intent, state.rootNode);
    if (!action) return;
    const before = copy(state.rootNode);
    try {
        withDerivedNodeIds(actionId, () => applyLayoutAction(state, action));
        if (state.rootNode) state.rootNode = balanceNode(state.rootNode);
    } catch {
        // The frontend's reducer throws where srv refuses; the tree it had stays.
        state.rootNode = before;
    }
}

describe("layout intent vectors (srv's results, replayed by the frontend's reducer)", () => {
    it("the vectors cover every intent", () => {
        const kinds = new Set(vectors.flatMap((v) => v.steps.map((s) => s.intent.type)));
        for (const k of ["insert", "insertatindex", "delete", "move", "swap", "resize", "replace", "splithorizontal", "splitvertical", "minimize", "restore"]) {
            expect(kinds).toContain(k);
        }
    });

    for (const v of vectors) {
        it(`seed ${v.seed}: the frontend predicts the tree srv stores`, () => {
            const state: LayoutTreeState = { rootNode: v.initial ? copy(v.initial) : undefined, pendingBackendActions: [] };
            for (const [i, s] of v.steps.entries()) {
                const before = copy(state.rootNode);
                step(state, s.actionId, s.intent);
                if (!s.applied) {
                    // srv refused it: the frontend must not have changed anything either.
                    expect(sameLayoutTree(state.rootNode, before), `step ${i} (${s.intent.type}) was refused by srv`).toBe(true);
                }
            }
            expect(sameLayoutTree(state.rootNode, v.final ?? undefined)).toBe(true);
        });
    }
});
