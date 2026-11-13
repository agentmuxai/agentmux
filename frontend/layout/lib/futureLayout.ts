// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Where a pane move leaves the layout, before it is dropped: the pending move is run on a copy of the tree
// by the same reducer the drop runs (`applyLayoutAction`), and the copy is laid out by the same layout pass
// (`computeNodeRects`). The ghost is the moved pane's rect in that result and the wireframe is every
// other pane's, so the preview is the drop by construction, whatever the tree: the old ghost was half of
// the target's rect in the layout as it stood, which ignored the dragged pane leaving its old place.

import { createMemo, untrack } from "solid-js";
import type { Properties as CSSProperties } from "csstype";
import { setTransform } from "./utils";
import { applyLayoutAction, copyLayoutNode } from "./layoutIntentActions";
import { computeNodeRects } from "./layoutGeometry";
import { withDerivedNodeIds } from "./layoutNode";
import type { LayoutModel } from "./layoutModel";
import { createTearOffPreview } from "./tearOffPreview";
import {
    LayoutTreeActionType,
    type LayoutNode,
    type LayoutTreeAction,
    type LayoutTreeDeleteNodeAction,
    type LayoutTreeMoveNodeAction,
    type LayoutTreeStayAction,
    type LayoutTreeSwapNodeAction,
} from "./types";

/** The layout a pending move leaves: every pane's rect, and which one is the moved pane. */
export interface FutureLayout {
    /** The id of the moved pane in the resulting tree, or of the pane that left it (`leaves`). */
    movedId: string;
    /** Set when the pane leaves the layout (a tear-off): it has no rect in the result. */
    leaves?: true;
    /** The rect of every node of the resulting tree, by node id. */
    rects: Record<string, Dimensions>;
    /** The ids of the resulting tree's panes (its leaves). */
    leafIds: string[];
    /**
     * Each of those panes' blockId, by node id. A pane keeps its node id through the preview, but a split can
     * give its parent a new one, so the live reflow matches a current pane to its future rect by block
     * (liveReflow.ts).
     */
    leafBlockIds: Record<string, string | undefined>;
}

function leavesOf(node: LayoutNode | undefined, out: LayoutNode[] = []): LayoutNode[] {
    if (!node) return out;
    if (node.children?.length) for (const c of node.children) leavesOf(c, out);
    else out.push(node);
    return out;
}

/** The node a move, swap or stay action carries, by its id, and none for any other action. */
function movedIdOf(action: LayoutTreeAction): string | undefined {
    if (action.type === LayoutTreeActionType.Move) return (action as LayoutTreeMoveNodeAction).node?.id;
    if (action.type === LayoutTreeActionType.Swap) return (action as LayoutTreeSwapNodeAction).node2Id;
    if (action.type === LayoutTreeActionType.Stay) return (action as LayoutTreeStayAction).nodeId;
    return undefined;
}

/**
 * The layout `action` (a pending Move, Swap or Stay) leaves, or undefined for anything else, for an empty
 * layout, or when the reducer refuses it. A Stay leaves the layout as it is, with the pane in its own rect.
 * The model's own tree is not touched.
 */
export function futureLayoutOf(model: LayoutModel, action: LayoutTreeAction | undefined): FutureLayout | undefined {
    const movedId = action ? movedIdOf(action) : undefined;
    const root = model.treeState.rootNode;
    if (!action || !movedId || !root) return undefined;
    try {
        // A moved node that is not in the tree (a tab pill's stand-in) is inserted as a copy.
        const run: LayoutTreeAction =
            action.type === LayoutTreeActionType.Move
                ? { ...action, node: copyLayoutNode((action as LayoutTreeMoveNodeAction).node) } as LayoutTreeMoveNodeAction
                : action;
        const after = laidOutAfter(model, root, run);
        return after?.rects[movedId] ? { movedId, ...after } : undefined;
    } catch (e) {
        console.warn("[layout] the drop preview could not be computed; falling back to the static ghost", e);
        return undefined;
    }
}

/** `action` run on a copy of the tree, and the copy laid out; none when it leaves no pane. */
function laidOutAfter(model: LayoutModel, root: LayoutNode, action: LayoutTreeAction): Omit<FutureLayout, "movedId"> | undefined {
    const state = { ...model.treeState, rootNode: copyLayoutNode(root) };
    // Names the branches the action creates, as a drop does (they do not change any rect). A Stay edits nothing.
    if (action.type !== LayoutTreeActionType.Stay) withDerivedNodeIds("preview", () => applyLayoutAction(state, action));
    if (!state.rootNode) return undefined;
    const leaves = leavesOf(state.rootNode);
    return {
        rects: computeNodeRects(model, state.rootNode),
        leafIds: leaves.map((n) => n.id),
        leafBlockIds: Object.fromEntries(leaves.map((n) => [n.id, n.data?.blockId])),
    };
}

/**
 * The layout that is left when the pane `nodeId` leaves it (a tear-off): every other pane's rect. None when
 * the pane is not in the tree or is its only pane. The model's own tree is not touched.
 */
export function layoutWithoutOf(model: LayoutModel, nodeId: string | undefined): FutureLayout | undefined {
    const root = model.treeState.rootNode;
    if (!nodeId || !root || !leavesOf(root).some((n) => n.id === nodeId)) return undefined;
    try {
        const gone = { type: LayoutTreeActionType.DeleteNode, nodeId } as LayoutTreeDeleteNodeAction;
        const after = laidOutAfter(model, root, gone);
        return after && { movedId: nodeId, leaves: true, ...after };
    } catch (e) {
        console.warn("[layout] the tear-off preview could not be computed", e);
        return undefined;
    }
}

/**
 * The layout as a drag sees it: its panes and their rects. A pending action is good for the layout it was
 * computed on and no other.
 */
function shapeOf(model: LayoutModel): string {
    const props = model.additionalProps();
    let shape = "";
    for (const leaf of model.leafs()) {
        const r = props[leaf.id]?.rect;
        shape += r ? `${leaf.id}:${leaf.data?.blockId}:${r.left},${r.top},${r.width},${r.height};` : `${leaf.id};`;
    }
    return shape;
}

/** Whether two pending actions are the same move: every field alike (ids, an index, the moved node itself). */
function sameAction(a: LayoutTreeAction, b: LayoutTreeAction): boolean {
    const [x, y] = [a as unknown as Record<string, unknown>, b as unknown as Record<string, unknown>];
    const keys = Object.keys(x);
    return keys.length === Object.keys(y).length && keys.every((k) => x[k] === y[k]);
}

/**
 * The memos a layout model keeps for the pending drag action: the layout it leaves, and the outline of
 * every pane of it but the moved one (the moved one is the ghost). Made in the model's reactive owner.
 *
 * The pending action is kept with the layout it was armed on. If the tree or a rect changes under the drag
 * (srv or another window closes a pane, the window is resized), the action describes a layout that is gone:
 * there is no future layout until the drag's next sample arms a candidate on the new one (the tracker starts
 * again on a changed layout, placementPreview.ts), so the live reflow lets every pane back to its own rect
 * instead of holding rects computed from a stale move (#47354).
 */
export function createPlacementPreview(model: LayoutModel) {
    // Every sample of a drag computes its action again and lays the tree out again (placementPreview.ts, the
    // reducer's `updateTree`): a new action and new rect objects each time, and a pointer held still goes on
    // being sampled. The same move on the same layout is the same armed drop, so the future layout is computed
    // once for it.
    const shape = createMemo(() => shapeOf(model));
    const armed = createMemo(
        () => {
            const action = model.pendingTreeAction.throttledValueAtom();
            return action ? { action, shape: untrack(shape) } : undefined;
        },
        undefined,
        { equals: (a, b) => a === b || (!!a && !!b && a.shape === b.shape && sameAction(a.action, b.action)) }
    );
    // The pending action while the layout is still the one it was armed on. A drop commits the tree before it
    // clears the action (the throttle can hold the clearing back a few ms more), and the action read against
    // the tree it has just made names other panes: the ghost's static fallback (`staticGhostTransform`) put
    // the landing zone on one of them for its last moment.
    const placementAction = createMemo(() => {
        const a = armed();
        return a && shape() === a.shape ? a.action : undefined;
    });
    const futureLayout = createMemo(() => {
        const action = placementAction();
        return action ? futureLayoutOf(model, action) : undefined;
    });
    const placementWireframe = createMemo((): { id: string; style: CSSProperties }[] => {
        const future = futureLayout();
        if (!future) return [];
        return future.leafIds
            .filter((id) => id !== future.movedId && future.rects[id])
            .map((id) => ({ id, style: setTransform(future.rects[id]) }));
    });
    // The layout a tear-off leaves, while the drag shows one (tearOffPreview.ts).
    const tearingOff = createTearOffPreview(model);
    const tearOffLayout = createMemo(() => {
        const nodeId = tearingOff();
        // Read again when the layout changes under it.
        return nodeId && shape() !== undefined ? layoutWithoutOf(model, nodeId) : undefined;
    });
    // Whether the drag that is on has shown yet where a release leaves the pane: a drop, or the tear-off
    // (the pointer rested over the pane's own place, or left the window). Until it has, the one thing a
    // release does is tear the pane off where it stands, and the pane's own place says so (tilelayout.scss
    // `.tear-off-offered`). Once the layout has shown anything else, that sign is over for the drag: it does
    // not come back when the pointer does, and the pane's place is drawn as any dragged pane's is (the
    // operator, 2026-11-12: "the red goes away once the user drags, when it comes back, red should not").
    const shownSomething = createMemo<boolean>((was) => (model.activeDrag() ? was || !!futureLayout() || !!tearOffLayout() : false), false);
    const tearOffOffered = () => model.activeDrag() && !shownSomething();
    const ownPlaceDrops = createOwnPlace(model, futureLayout, tearOffLayout);
    return { placementAction, futureLayout, placementWireframe, tearOffLayout, tearOffOffered, ownPlaceDrops };
}

/**
 * Whether the dragged pane's own place is a drop like any other: the layout shows the pane there under the
 * landing zone, and a release puts it back. It starts as the place a release tears the pane off, since every
 * drag starts over it, and stays that through a tear-off the layout is showing (the pointer rested there). It
 * becomes an ordinary drop once the user has gone on to something else: a drop was shown, or a tear-off was
 * and is no longer (the pointer went on to another pane, or left the window and came back). From then on the
 * pane is torn off by a release outside the window, as any pane is (the operator, 2026-11-12: "it is suppose
 * to behave exactly like an ordinary pane, once they decide not to tear off and it rearranges").
 *
 * Kept as it was when the drag is released, until the next one starts: the release is handled after the
 * source has ended the drag (the source's onDrop comes first), and must still know which it was.
 */
function createOwnPlace(model: LayoutModel, futureLayout: () => unknown, tearOffLayout: () => unknown): () => boolean {
    const state = createMemo<{ active: boolean; tornOff: boolean; drops: boolean }>(
        (was) => {
            if (!model.activeDrag()) return was.active ? { ...was, active: false } : was;
            const from = was.active ? was : { tornOff: false, drops: false };
            const tearOff = !!tearOffLayout();
            const drops = from.drops || !!futureLayout() || (from.tornOff && !tearOff);
            return { active: true, tornOff: from.tornOff || tearOff, drops };
        },
        { active: false, tornOff: false, drops: false }
    );
    return () => state().drops;
}
