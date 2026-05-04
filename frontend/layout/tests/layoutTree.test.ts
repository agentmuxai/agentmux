// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { assert, test } from "vitest";
import { newLayoutNode } from "../lib/layoutNode";
import {
    clearTree,
    computeMoveNode,
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
} from "../lib/layoutTree";
import {
    DropDirection,
    FlexDirection,
    LayoutTreeActionType,
    LayoutTreeComputeMoveNodeAction,
    LayoutTreeMoveNodeAction,
} from "../lib/types";
import { newLayoutTreeState } from "./model";

test("layoutTreeStateReducer - compute move", () => {
    const node1 = newLayoutNode(undefined, undefined, undefined, { blockId: "node1" });
    const node2 = newLayoutNode(undefined, undefined, undefined, { blockId: "node2" });
    const node3 = newLayoutNode(undefined, undefined, undefined, { blockId: "node3" });
    const rootNode = newLayoutNode(undefined, undefined, [node1, node2, node3], undefined);
    const treeState = newLayoutTreeState(rootNode);

    // Move node2 ahead of node1.
    let pendingAction = computeMoveNode(treeState, {
        type: LayoutTreeActionType.ComputeMove,
        nodeId: node1.id,
        nodeToMoveId: node2.id,
        direction: DropDirection.Top,
    });
    const moveOp = pendingAction as LayoutTreeMoveNodeAction;
    assert(moveOp, "computeMoveNode should return a move operation");
    assert(moveOp.parentId === treeState.rootNode.id, "move operation should target the root node");
    assert(moveOp.index === 0, "node2 should be inserted at the beginning");
    moveNode(treeState, moveOp);
    assert(treeState.rootNode.children![0].id === node2.id, "node2 should now be first child");
    assert(treeState.rootNode.children![1].id === node1.id, "node1 should now follow node2");

    // Move node2 to the end of the root list.
    pendingAction = computeMoveNode(treeState, {
        type: LayoutTreeActionType.ComputeMove,
        nodeId: node3.id,
        nodeToMoveId: node2.id,
        direction: DropDirection.Bottom,
    });
    const moveOpBottom = pendingAction as LayoutTreeMoveNodeAction;
    assert(moveOpBottom, "computeMoveNode should produce a second move operation");
    assert(
        moveOpBottom.parentId === treeState.rootNode.id,
        "move operation should target the root parent when dropping below node3"
    );
    moveNode(treeState, moveOpBottom);
    const children = treeState.rootNode.children!;
    assert(children[0].id === node1.id, "node1 should become the first child after node2 moves away");
    assert(children[1].id === node3.id, "node3 should remain in the middle position");
    assert(children[2].id === node2.id, "node2 should be reinserted at the end after dropping below node3");
});

test("computeMove - noop action", () => {
    let nodeToMove = newLayoutNode(undefined, undefined, undefined, { blockId: "nodeToMove" });
    let treeState = newLayoutTreeState(
        newLayoutNode(undefined, undefined, [
            nodeToMove,
            newLayoutNode(undefined, undefined, undefined, { blockId: "otherNode" }),
        ])
    );
    let moveAction: LayoutTreeComputeMoveNodeAction = {
        type: LayoutTreeActionType.ComputeMove,
        nodeId: treeState.rootNode.id,
        nodeToMoveId: nodeToMove.id,
        direction: DropDirection.Left,
    };
    let pendingAction = computeMoveNode(treeState, moveAction);

    assert(pendingAction === undefined, "inserting a node to the left of itself should not produce a pendingAction");

    moveAction = {
        type: LayoutTreeActionType.ComputeMove,
        nodeId: treeState.rootNode.id,
        nodeToMoveId: nodeToMove.id,
        direction: DropDirection.Right,
    };

    pendingAction = computeMoveNode(treeState, moveAction);
    assert(pendingAction === undefined, "inserting a node to the right of itself should not produce a pendingAction");
});

// ── insertNode ────────────────────────────────────────────────────────────────

test("insertNode - empty tree promotes node to root", () => {
    const treeState = newLayoutTreeState(undefined as any);
    const newNode = newLayoutNode(undefined, undefined, undefined, { blockId: "first" });
    insertNode(treeState, { type: LayoutTreeActionType.InsertNode, node: newNode });
    assert(treeState.rootNode === newNode, "inserting into empty tree sets the node as root");
});

test("insertNode - existing tree appends via findNextInsertLocation", () => {
    const child = newLayoutNode(undefined, undefined, undefined, { blockId: "child" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [child]));
    const newNode = newLayoutNode(undefined, undefined, undefined, { blockId: "second" });
    insertNode(treeState, { type: LayoutTreeActionType.InsertNode, node: newNode });
    assert(treeState.rootNode.children!.some((c) => c.id === newNode.id), "new node should be inserted into the tree");
});

test("insertNode - magnified flag also focuses the node", () => {
    const child = newLayoutNode(undefined, undefined, undefined, { blockId: "child" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [child]));
    const newNode = newLayoutNode(undefined, undefined, undefined, { blockId: "magnified" });
    insertNode(treeState, {
        type: LayoutTreeActionType.InsertNode,
        node: newNode,
        magnified: true,
    });
    assert(treeState.magnifiedNodeId === newNode.id, "magnified flag sets magnifiedNodeId");
    assert(treeState.focusedNodeId === newNode.id, "magnified flag also sets focusedNodeId (cohesion)");
});

test("insertNode - focused-only flag", () => {
    const child = newLayoutNode(undefined, undefined, undefined, { blockId: "child" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [child]));
    const newNode = newLayoutNode(undefined, undefined, undefined, { blockId: "focused" });
    insertNode(treeState, {
        type: LayoutTreeActionType.InsertNode,
        node: newNode,
        focused: true,
    });
    assert(treeState.focusedNodeId === newNode.id);
    assert(treeState.magnifiedNodeId === undefined, "focused alone does not magnify");
});

test("insertNode - missing node is a no-op (logs error, does not crash)", () => {
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, undefined, { blockId: "root" }));
    const before = JSON.stringify(treeState);
    insertNode(treeState, { type: LayoutTreeActionType.InsertNode, node: undefined as any });
    assert(JSON.stringify(treeState) === before, "missing node should leave state unchanged");
});

// ── insertNodeAtIndex ─────────────────────────────────────────────────────────

test("insertNodeAtIndex - missing indexArr is a no-op", () => {
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, undefined, { blockId: "root" }));
    const newNode = newLayoutNode(undefined, undefined, undefined, { blockId: "n" });
    const before = JSON.stringify(treeState);
    insertNodeAtIndex(treeState, {
        type: LayoutTreeActionType.InsertNodeAtIndex,
        node: newNode,
        indexArr: undefined as any,
    });
    assert(JSON.stringify(treeState) === before, "missing indexArr should leave state unchanged");
});

test("insertNodeAtIndex - empty tree promotes node to root", () => {
    const treeState = newLayoutTreeState(undefined as any);
    const newNode = newLayoutNode(undefined, undefined, undefined, { blockId: "first" });
    insertNodeAtIndex(treeState, {
        type: LayoutTreeActionType.InsertNodeAtIndex,
        node: newNode,
        indexArr: [0],
    });
    assert(treeState.rootNode === newNode);
});

// ── swapNode ──────────────────────────────────────────────────────────────────

test("swapNode - swaps two siblings (positions and sizes)", () => {
    const node1 = newLayoutNode(undefined, 30, undefined, { blockId: "n1" });
    const node2 = newLayoutNode(undefined, 70, undefined, { blockId: "n2" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [node1, node2]));
    swapNode(treeState, {
        type: LayoutTreeActionType.Swap,
        node1Id: node1.id,
        node2Id: node2.id,
    });
    assert(treeState.rootNode.children![0].id === node2.id, "node2 takes node1's slot");
    assert(treeState.rootNode.children![1].id === node1.id, "node1 takes node2's slot");
    assert(treeState.rootNode.children![0].size === 30, "size at slot 0 stays 30 (sizes swap with the nodes)");
    assert(treeState.rootNode.children![1].size === 70, "size at slot 1 stays 70");
});

test("swapNode - rejects swapping the root node", () => {
    const child = newLayoutNode(undefined, undefined, undefined, { blockId: "child" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [child]));
    const before = JSON.stringify(treeState);
    swapNode(treeState, {
        type: LayoutTreeActionType.Swap,
        node1Id: treeState.rootNode.id,
        node2Id: child.id,
    });
    assert(JSON.stringify(treeState) === before, "cannot swap root; state unchanged");
});

test("swapNode - rejects swapping a node with itself", () => {
    const node1 = newLayoutNode(undefined, undefined, undefined, { blockId: "n1" });
    const node2 = newLayoutNode(undefined, undefined, undefined, { blockId: "n2" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [node1, node2]));
    const before = JSON.stringify(treeState);
    swapNode(treeState, {
        type: LayoutTreeActionType.Swap,
        node1Id: node1.id,
        node2Id: node1.id,
    });
    assert(JSON.stringify(treeState) === before, "self-swap is a no-op");
});

// ── deleteNode ────────────────────────────────────────────────────────────────

test("deleteNode - removes a child", () => {
    const node1 = newLayoutNode(undefined, undefined, undefined, { blockId: "keep" });
    const node2 = newLayoutNode(undefined, undefined, undefined, { blockId: "drop" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [node1, node2]));
    deleteNode(treeState, { type: LayoutTreeActionType.DeleteNode, nodeId: node2.id });
    assert(treeState.rootNode.children!.length === 1);
    assert(treeState.rootNode.children![0].id === node1.id);
});

test("deleteNode - clears focusedNodeId when the focused node is deleted", () => {
    const node1 = newLayoutNode(undefined, undefined, undefined, { blockId: "n1" });
    const node2 = newLayoutNode(undefined, undefined, undefined, { blockId: "n2" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [node1, node2]));
    treeState.focusedNodeId = node2.id;
    deleteNode(treeState, { type: LayoutTreeActionType.DeleteNode, nodeId: node2.id });
    assert(treeState.focusedNodeId === undefined, "deleting the focused node clears focus");
});

test("deleteNode - deleting non-focused node leaves focus intact", () => {
    const node1 = newLayoutNode(undefined, undefined, undefined, { blockId: "n1" });
    const node2 = newLayoutNode(undefined, undefined, undefined, { blockId: "n2" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [node1, node2]));
    treeState.focusedNodeId = node1.id;
    deleteNode(treeState, { type: LayoutTreeActionType.DeleteNode, nodeId: node2.id });
    assert(treeState.focusedNodeId === node1.id);
});

test("deleteNode - deleting the root sets rootNode to undefined", () => {
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, undefined, { blockId: "root" }));
    deleteNode(treeState, {
        type: LayoutTreeActionType.DeleteNode,
        nodeId: treeState.rootNode.id,
    });
    assert(treeState.rootNode === undefined);
});

// ── resizeNode ────────────────────────────────────────────────────────────────

test("resizeNode - applies size to the matching node", () => {
    const node1 = newLayoutNode(undefined, 50, undefined, { blockId: "n1" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [node1]));
    resizeNode(treeState, {
        type: LayoutTreeActionType.ResizeNode,
        resizeOperations: [{ nodeId: node1.id, size: 80 }],
    });
    assert(treeState.rootNode.children![0].size === 80);
});

test("resizeNode - rejects size out of bounds (early return at first invalid op)", () => {
    const node1 = newLayoutNode(undefined, 50, undefined, { blockId: "n1" });
    const node2 = newLayoutNode(undefined, 50, undefined, { blockId: "n2" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [node1, node2]));
    resizeNode(treeState, {
        type: LayoutTreeActionType.ResizeNode,
        resizeOperations: [
            { nodeId: node1.id, size: 150 }, // invalid → bails before touching either node
            { nodeId: node2.id, size: 25 },
        ],
    });
    // No mutation: the first invalid op causes an early return BEFORE applying.
    assert(node1.size === 50, "n1 should be untouched (out-of-range guard ran first)");
    assert(node2.size === 50, "n2 should be untouched (loop bailed)");
});

// ── focusNode ────────────────────────────────────────────────────────────────

test("focusNode - sets focusedNodeId", () => {
    const node1 = newLayoutNode(undefined, undefined, undefined, { blockId: "n1" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [node1]));
    focusNode(treeState, { type: LayoutTreeActionType.FocusNode, nodeId: node1.id });
    assert(treeState.focusedNodeId === node1.id);
});

test("focusNode - missing nodeId is a no-op", () => {
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, undefined, { blockId: "root" }));
    treeState.focusedNodeId = "preexisting";
    focusNode(treeState, { type: LayoutTreeActionType.FocusNode, nodeId: undefined as any });
    assert(treeState.focusedNodeId === "preexisting", "missing nodeId leaves state unchanged");
});

// ── magnifyNodeToggle ────────────────────────────────────────────────────────

test("magnifyNodeToggle - toggles a node ON, also focuses it", () => {
    const child = newLayoutNode(undefined, undefined, undefined, { blockId: "child" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [child]));
    magnifyNodeToggle(treeState, {
        type: LayoutTreeActionType.MagnifyNodeToggle,
        nodeId: child.id,
    });
    assert(treeState.magnifiedNodeId === child.id);
    assert(treeState.focusedNodeId === child.id, "magnify toggle ON also sets focus (cohesion)");
});

test("magnifyNodeToggle - second toggle clears (does NOT clear focus)", () => {
    const child = newLayoutNode(undefined, undefined, undefined, { blockId: "child" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [child]));
    magnifyNodeToggle(treeState, {
        type: LayoutTreeActionType.MagnifyNodeToggle,
        nodeId: child.id,
    });
    magnifyNodeToggle(treeState, {
        type: LayoutTreeActionType.MagnifyNodeToggle,
        nodeId: child.id,
    });
    assert(treeState.magnifiedNodeId === undefined);
    // Subtle: the OFF branch doesn't touch focusedNodeId — focus persists from the ON branch.
    assert(treeState.focusedNodeId === child.id, "OFF branch preserves focus from ON branch");
});

test("magnifyNodeToggle - rejects magnifying the root node", () => {
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, undefined, { blockId: "root" }));
    magnifyNodeToggle(treeState, {
        type: LayoutTreeActionType.MagnifyNodeToggle,
        nodeId: treeState.rootNode.id,
    });
    assert(treeState.magnifiedNodeId === undefined, "root cannot be magnified");
});

test("magnifyNodeToggle - missing nodeId is a no-op", () => {
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, undefined, { blockId: "root" }));
    treeState.magnifiedNodeId = "preexisting";
    magnifyNodeToggle(treeState, {
        type: LayoutTreeActionType.MagnifyNodeToggle,
        nodeId: undefined as any,
    });
    assert(treeState.magnifiedNodeId === "preexisting");
});

// ── clearTree ────────────────────────────────────────────────────────────────

test("clearTree - resets all four fields", () => {
    const child = newLayoutNode(undefined, undefined, undefined, { blockId: "child" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [child]));
    treeState.focusedNodeId = child.id;
    treeState.magnifiedNodeId = child.id;
    treeState.leafOrder = [{ nodeid: child.id, blockid: "child" }];
    clearTree(treeState);
    assert(treeState.rootNode === undefined);
    assert(treeState.focusedNodeId === undefined);
    assert(treeState.magnifiedNodeId === undefined);
    assert(treeState.leafOrder === undefined);
});

// ── replaceNode ──────────────────────────────────────────────────────────────

test("replaceNode - replaces root, preserving size", () => {
    const root = newLayoutNode(undefined, 100, undefined, { blockId: "old-root" });
    root.size = 100;
    const treeState = newLayoutTreeState(root);
    const newRoot = newLayoutNode(undefined, 50, undefined, { blockId: "new-root" });
    replaceNode(treeState, {
        type: LayoutTreeActionType.ReplaceNode,
        targetNodeId: root.id,
        newNode: newRoot,
    });
    assert(treeState.rootNode === newRoot);
    assert(treeState.rootNode.size === 100, "replaceNode preserves the old node's size");
});

test("replaceNode - replaces a child in-place, preserving size", () => {
    const child1 = newLayoutNode(undefined, 30, undefined, { blockId: "c1" });
    const child2 = newLayoutNode(undefined, 70, undefined, { blockId: "c2" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [child1, child2]));
    const replacement = newLayoutNode(undefined, 99, undefined, { blockId: "replacement" });
    replaceNode(treeState, {
        type: LayoutTreeActionType.ReplaceNode,
        targetNodeId: child2.id,
        newNode: replacement,
    });
    assert(treeState.rootNode.children![1].id === replacement.id);
    assert(treeState.rootNode.children![1].size === 70, "replacement inherits child2's size");
});

test("replaceNode - focused flag updates focusedNodeId", () => {
    const child = newLayoutNode(undefined, undefined, undefined, { blockId: "child" });
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, [child]));
    const replacement = newLayoutNode(undefined, undefined, undefined, { blockId: "rep" });
    replaceNode(treeState, {
        type: LayoutTreeActionType.ReplaceNode,
        targetNodeId: child.id,
        newNode: replacement,
        focused: true,
    });
    assert(treeState.focusedNodeId === replacement.id);
});

// ── splitHorizontal / splitVertical ──────────────────────────────────────────

test("splitHorizontal - inserts after the target in a Row parent", () => {
    const child = newLayoutNode(undefined, undefined, undefined, { blockId: "c1" });
    // Row parent
    const root = newLayoutNode(FlexDirection.Row, undefined, [child]);
    const treeState = newLayoutTreeState(root);
    const newNode = newLayoutNode(undefined, undefined, undefined, { blockId: "new" });
    splitHorizontal(treeState, {
        type: LayoutTreeActionType.SplitHorizontal,
        targetNodeId: child.id,
        newNode,
        position: "after",
    });
    assert(treeState.rootNode.children!.length === 2);
    assert(treeState.rootNode.children![1].id === newNode.id, "new node inserted after target");
});

test("splitHorizontal - inserts before the target in a Row parent", () => {
    const child = newLayoutNode(undefined, undefined, undefined, { blockId: "c1" });
    const root = newLayoutNode(FlexDirection.Row, undefined, [child]);
    const treeState = newLayoutTreeState(root);
    const newNode = newLayoutNode(undefined, undefined, undefined, { blockId: "new" });
    splitHorizontal(treeState, {
        type: LayoutTreeActionType.SplitHorizontal,
        targetNodeId: child.id,
        newNode,
        position: "before",
    });
    assert(treeState.rootNode.children![0].id === newNode.id);
});

test("splitVertical - wraps target when parent is not Column", () => {
    // Root has no parent and is the leaf itself — splitVertical should wrap.
    const leaf = newLayoutNode(undefined, undefined, undefined, { blockId: "only" });
    const treeState = newLayoutTreeState(leaf);
    const newNode = newLayoutNode(undefined, undefined, undefined, { blockId: "new" });
    splitVertical(treeState, {
        type: LayoutTreeActionType.SplitVertical,
        targetNodeId: leaf.id,
        newNode,
        position: "after",
    });
    // Root should now be a wrapping Column with the original leaf + newNode as children.
    assert(treeState.rootNode.flexDirection === FlexDirection.Column, "root wrapped in Column");
    assert(treeState.rootNode.children!.length === 2);
});

test("splitHorizontal - missing target is a no-op (logs error)", () => {
    const treeState = newLayoutTreeState(newLayoutNode(undefined, undefined, undefined, { blockId: "root" }));
    const newNode = newLayoutNode(undefined, undefined, undefined, { blockId: "new" });
    const before = JSON.stringify(treeState);
    splitHorizontal(treeState, {
        type: LayoutTreeActionType.SplitHorizontal,
        targetNodeId: "ghost-id",
        newNode,
        position: "after",
    });
    assert(JSON.stringify(treeState) === before);
});
