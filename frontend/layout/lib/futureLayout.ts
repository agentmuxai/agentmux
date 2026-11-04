// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Where a pane move leaves the layout, before it is dropped: the pending move is run on a copy of the tree
// by the same reducer the drop runs (`applyLayoutAction`), and the copy is laid out by the same layout pass
// (`computeNodeRects`). The ghost is the moved pane's rect in that result and the wireframe is every
// other pane's, so the preview is the drop by construction, whatever the tree: the old ghost was half of
// the target's rect in the layout as it stood, which ignored the dragged pane leaving its old place.

import { createMemo } from "solid-js";
import type { Properties as CSSProperties } from "csstype";
import { setTransform } from "./utils";
import { applyLayoutAction, copyLayoutNode } from "./layoutIntentActions";
import { computeNodeRects } from "./layoutGeometry";
import { withDerivedNodeIds } from "./layoutNode";
import type { LayoutModel } from "./layoutModel";
import {
    LayoutTreeActionType,
    type LayoutNode,
    type LayoutTreeAction,
    type LayoutTreeMoveNodeAction,
    type LayoutTreeSwapNodeAction,
} from "./types";

/** The layout a pending move leaves: every pane's rect, and which one is the moved pane. */
export interface FutureLayout {
    /** The id of the moved pane in the resulting tree. */
    movedId: string;
    /** The rect of every node of the resulting tree, by node id. */
    rects: Record<string, Dimensions>;
    /** The ids of the resulting tree's panes (its leaves). */
    leafIds: string[];
}

function leavesOf(node: LayoutNode | undefined, out: string[] = []): string[] {
    if (!node) return out;
    if (node.children?.length) for (const c of node.children) leavesOf(c, out);
    else out.push(node.id);
    return out;
}

/** The node a move or swap action carries, by its id, and none for any other action. */
function movedIdOf(action: LayoutTreeAction): string | undefined {
    if (action.type === LayoutTreeActionType.Move) return (action as LayoutTreeMoveNodeAction).node?.id;
    if (action.type === LayoutTreeActionType.Swap) return (action as LayoutTreeSwapNodeAction).node2Id;
    return undefined;
}

/**
 * The layout `action` (a pending Move or Swap) leaves, or undefined for anything else, for an empty
 * layout, or when the reducer refuses it. The model's own tree is not touched.
 */
export function futureLayoutOf(model: LayoutModel, action: LayoutTreeAction | undefined): FutureLayout | undefined {
    const movedId = action ? movedIdOf(action) : undefined;
    const root = model.treeState.rootNode;
    if (!action || !movedId || !root) return undefined;
    try {
        const copy = copyLayoutNode(root);
        const state = { ...model.treeState, rootNode: copy };
        // Names the branches the move creates, as a drop does (they do not change any rect).
        // A moved node that is not in the tree (a tab pill's stand-in) is inserted as a copy.
        const run: LayoutTreeAction =
            action.type === LayoutTreeActionType.Move
                ? { ...action, node: copyLayoutNode((action as LayoutTreeMoveNodeAction).node) } as LayoutTreeMoveNodeAction
                : action;
        withDerivedNodeIds("preview", () => applyLayoutAction(state, run));
        const rects = computeNodeRects(model, state.rootNode);
        if (!rects[movedId]) return undefined;
        return { movedId, rects, leafIds: leavesOf(state.rootNode) };
    } catch (e) {
        console.warn("[layout] the drop preview could not be computed; falling back to the static ghost", e);
        return undefined;
    }
}

/**
 * The memos a layout model keeps for the pending drag action: the layout it leaves, and the outline of
 * every pane of it but the moved one (the moved one is the ghost). Made in the model's reactive owner.
 */
export function createPlacementPreview(model: LayoutModel) {
    const futureLayout = createMemo(() => futureLayoutOf(model, model.pendingTreeAction.throttledValueAtom()));
    const placementWireframe = createMemo((): { id: string; style: CSSProperties }[] => {
        const future = futureLayout();
        if (!future) return [];
        return future.leafIds
            .filter((id) => id !== future.movedId && future.rects[id])
            .map((id) => ({ id, style: setTransform(future.rects[id]) }));
    });
    return { futureLayout, placementWireframe };
}
