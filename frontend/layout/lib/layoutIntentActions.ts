// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pure half of layout intents (layoutIntents.ts): which tree actions are
 * intents, the intent for an action, the frontend's own application of one,
 * and comparing a tree with srv's. No store or RPC imports, so the vector test
 * runs it bare. SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §4.
 */

import type { LayoutIntent } from "@/types/rpc/LayoutIntent";
import {
    clearTree,
    deleteNode,
    focusNode,
    insertNode,
    insertNodeAtIndex,
    magnifyNodeToggle,
    moveNode,
    replaceNode,
    resizeNode,
    splitHorizontal,
    splitVertical,
    swapNode,
} from "./layoutTree";
import {
    LayoutNode,
    LayoutTreeAction,
    LayoutTreeActionType,
    LayoutTreeDeleteNodeAction,
    LayoutTreeFocusNodeAction,
    LayoutTreeInsertNodeAction,
    LayoutTreeInsertNodeAtIndexAction,
    LayoutTreeMagnifyNodeToggleAction,
    LayoutTreeMoveNodeAction,
    LayoutTreeReplaceNodeAction,
    LayoutTreeResizeNodeAction,
    LayoutTreeSplitHorizontalAction,
    LayoutTreeSplitVerticalAction,
    LayoutTreeState,
    LayoutTreeSwapNodeAction,
} from "./types";

/** The actions srv applies as intents. */
const INTENT_ACTIONS = new Set<LayoutTreeActionType>([
    LayoutTreeActionType.Move,
    LayoutTreeActionType.Swap,
    LayoutTreeActionType.ResizeNode,
    LayoutTreeActionType.InsertNode,
    LayoutTreeActionType.InsertNodeAtIndex,
    LayoutTreeActionType.DeleteNode,
    LayoutTreeActionType.FocusNode,
    LayoutTreeActionType.MagnifyNodeToggle,
    LayoutTreeActionType.ClearTree,
    LayoutTreeActionType.ReplaceNode,
    LayoutTreeActionType.SplitHorizontal,
    LayoutTreeActionType.SplitVertical,
]);

export function isIntentAction(type: LayoutTreeActionType): boolean {
    return INTENT_ACTIONS.has(type);
}

export const copyLayoutNode = (node: LayoutNode): LayoutNode => JSON.parse(JSON.stringify(node));

/**
 * The intent for a tree action. Call it BEFORE applying the action: applying
 * edits the nodes it carries (a size carved off, a direction flipped), and
 * `insertNodeAtIndex` consumes its index path.
 */
export function intentForAction(action: LayoutTreeAction): LayoutIntent | null {
    switch (action.type) {
        case LayoutTreeActionType.Move: {
            const a = action as LayoutTreeMoveNodeAction;
            return { type: "move", nodeId: a.node.id, parentId: a.parentId, index: a.index, insertAtRoot: !!a.insertAtRoot };
        }
        case LayoutTreeActionType.Swap: {
            const a = action as LayoutTreeSwapNodeAction;
            return { type: "swap", node1Id: a.node1Id, node2Id: a.node2Id };
        }
        case LayoutTreeActionType.ResizeNode: {
            const a = action as LayoutTreeResizeNodeAction;
            return { type: "resize", resizeOperations: (a.resizeOperations ?? []).map((op) => ({ nodeId: op.nodeId, size: op.size })) };
        }
        case LayoutTreeActionType.InsertNode: {
            const a = action as LayoutTreeInsertNodeAction;
            return { type: "insert", node: copyLayoutNode(a.node), magnified: !!a.magnified, focused: !!a.focused };
        }
        case LayoutTreeActionType.InsertNodeAtIndex: {
            const a = action as LayoutTreeInsertNodeAtIndexAction;
            return {
                type: "insertatindex",
                node: copyLayoutNode(a.node),
                indexArr: [...(a.indexArr ?? [])],
                magnified: !!a.magnified,
                focused: !!a.focused,
            };
        }
        case LayoutTreeActionType.DeleteNode:
            return { type: "delete", nodeId: (action as LayoutTreeDeleteNodeAction).nodeId };
        case LayoutTreeActionType.ReplaceNode: {
            const a = action as LayoutTreeReplaceNodeAction;
            return { type: "replace", targetNodeId: a.targetNodeId, newNode: copyLayoutNode(a.newNode), focused: !!a.focused };
        }
        case LayoutTreeActionType.SplitHorizontal:
        case LayoutTreeActionType.SplitVertical: {
            const a = action as LayoutTreeSplitHorizontalAction | LayoutTreeSplitVerticalAction;
            return {
                type: action.type === LayoutTreeActionType.SplitHorizontal ? "splithorizontal" : "splitvertical",
                targetNodeId: a.targetNodeId,
                newNode: copyLayoutNode(a.newNode),
                position: a.position,
                sizeFraction: a.sizeFraction,
                focused: !!a.focused,
            };
        }
        case LayoutTreeActionType.ClearTree:
            return { type: "clear" };
        case LayoutTreeActionType.FocusNode:
        case LayoutTreeActionType.MagnifyNodeToggle:
            return { type: "view" };
        default:
            return null;
    }
}

/** Apply an intent action to `state` with the frontend's own tree functions —
 *  the prediction of what srv will store. */
export function applyLayoutAction(state: LayoutTreeState, action: LayoutTreeAction): void {
    switch (action.type) {
        case LayoutTreeActionType.Move:
            moveNode(state, action as LayoutTreeMoveNodeAction);
            break;
        case LayoutTreeActionType.InsertNode:
            insertNode(state, action as LayoutTreeInsertNodeAction);
            break;
        case LayoutTreeActionType.InsertNodeAtIndex:
            insertNodeAtIndex(state, action as LayoutTreeInsertNodeAtIndexAction);
            break;
        case LayoutTreeActionType.DeleteNode:
            deleteNode(state, action as LayoutTreeDeleteNodeAction);
            break;
        case LayoutTreeActionType.Swap:
            swapNode(state, action as LayoutTreeSwapNodeAction);
            break;
        case LayoutTreeActionType.ResizeNode:
            resizeNode(state, action as LayoutTreeResizeNodeAction);
            break;
        case LayoutTreeActionType.FocusNode:
            focusNode(state, action as LayoutTreeFocusNodeAction);
            break;
        case LayoutTreeActionType.MagnifyNodeToggle:
            magnifyNodeToggle(state, action as LayoutTreeMagnifyNodeToggleAction);
            break;
        case LayoutTreeActionType.ClearTree:
            clearTree(state);
            break;
        case LayoutTreeActionType.ReplaceNode:
            replaceNode(state, action as LayoutTreeReplaceNodeAction);
            break;
        case LayoutTreeActionType.SplitHorizontal:
            splitHorizontal(state, action as LayoutTreeSplitHorizontalAction);
            break;
        case LayoutTreeActionType.SplitVertical:
            splitVertical(state, action as LayoutTreeSplitVerticalAction);
            break;
    }
}

// ── Comparing with srv's tree ──────────────────────────────────────────────

/** A node as plain JSON, sizes to 6 significant digits (srv stores f32), with
 *  absent and empty optional fields dropped. */
function canonical(node: LayoutNode | undefined): unknown {
    if (!node) return null;
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(node).sort()) {
        const value = (node as unknown as Record<string, unknown>)[key];
        if (value === undefined || value === null) continue;
        if (key === "children") {
            if ((value as LayoutNode[]).length) out.children = (value as LayoutNode[]).map(canonical);
        } else if (key === "size") {
            out.size = Number((value as number).toPrecision(6));
        } else if (key === "data") {
            const data: Record<string, unknown> = {};
            for (const k of Object.keys(value as object).sort()) {
                const v = (value as Record<string, unknown>)[k];
                if (v === undefined || v === null || v === "" || (Array.isArray(v) && !v.length)) continue;
                data[k] = v;
            }
            out.data = data;
        } else {
            out[key] = value;
        }
    }
    return out;
}

/** Whether two trees are the same layout (ids, structure, sizes, data, flags). */
export function sameLayoutTree(a: LayoutNode | undefined, b: LayoutNode | undefined): boolean {
    return JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
}
