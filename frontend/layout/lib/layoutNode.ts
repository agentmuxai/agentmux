// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { DEFAULT_MAX_CHILDREN } from "./layoutTree";
import { isEffectivelyMinimized } from "./layoutInvariants";
import { DefaultNodeSize, FlexDirection, LayoutNode } from "./types";
import { reverseFlexDirection } from "./utils";

/**
 * Creates a new node.
 * @param flexDirection The flex direction for the new node.
 * @param size The size for the new node.
 * @param children The children for the new node.
 * @param data The data for the new node.
 * @returns The new node.
 */
export function newLayoutNode(
    flexDirection?: FlexDirection,
    size?: number,
    children?: LayoutNode[],
    data?: TabLayoutData
): LayoutNode {
    const newNode: LayoutNode = {
        id: crypto.randomUUID(),
        flexDirection: flexDirection ?? FlexDirection.Row,
        size: size ?? DefaultNodeSize,
        children,
        data,
    };

    if (!validateNode(newNode)) {
        throw new Error("Invalid node");
    }
    return newNode;
}

/**
 * Adds new nodes to the tree at the given index.
 * @param node The parent node.
 * @param idx The index to insert at.
 * @param children The nodes to insert.
 * @returns The updated parent node.
 */
export function addChildAt(node: LayoutNode, idx: number, ...children: LayoutNode[]) {
    // console.log("adding", children, "to", node, "at index", idx);
    if (children.length === 0) return;

    if (!node.children) {
        addIntermediateNode(node);
    }
    const childrenToAdd = children.flatMap((v) => {
        if (v.flexDirection !== node.flexDirection) {
            return v;
        } else if (v.children) {
            return v.children;
        } else {
            v.flexDirection = reverseFlexDirection(node.flexDirection);
            return v;
        }
    });

    if (node.children.length <= idx) {
        node.children.push(...childrenToAdd);
    } else if (idx >= 0) {
        node.children.splice(idx, 0, ...childrenToAdd);
    }
}

/**
 * Adds an intermediate node as a direct child of the given node, moving the given node's children or data into it.
 *
 * If the node contains children, they are moved two levels deeper to preserve their flex direction. If the node only has data, it is moved one level deeper.
 * @param node The node to add the intermediate node to.
 * @returns The updated node and the node that was added.
 */
export function addIntermediateNode(node: LayoutNode): LayoutNode {
    let intermediateNode: LayoutNode;

    if (node.data) {
        intermediateNode = newLayoutNode(reverseFlexDirection(node.flexDirection), undefined, undefined, node.data);
        node.children = [intermediateNode];
        node.data = undefined;
        // Defense-in-depth: callers are expected to guard against promoting
        // a minimized leaf (see findNextInsertLocation/insertNode's own
        // isEffectivelyMinimized checks), but if this ever runs on one
        // anyway, `minimized` must travel with the DATA, not stay on `node`
        // — `node` is now a branch (leaf-only field; the 2026-07-18
        // "expanded pane shows Restore" corruption was exactly a leaf's
        // `minimized` flag left stranded on the branch it got promoted
        // into, while the intermediate child that inherited the leaf's id
        // silently lost it).
        if (node.minimized) {
            intermediateNode.minimized = true;
            node.minimized = undefined;
        }
    } else {
        const intermediateNodeInner = newLayoutNode(node.flexDirection, undefined, node.children);
        intermediateNode = newLayoutNode(reverseFlexDirection(node.flexDirection), undefined, [intermediateNodeInner]);
        node.children = [intermediateNode];
    }
    const intermediateNodeId = intermediateNode.id;
    intermediateNode.id = node.id;
    node.id = intermediateNodeId;
    return intermediateNode;
}

/**
 * Attempts to remove the specified node from its parent.
 * @param parent The parent node.
 * @param childToRemove The node to remove.
 * @param startingIndex The index in children to start the search from.
 * @returns The updated parent node, or undefined if the node was not found.
 */
export function removeChild(parent: LayoutNode, childToRemove: LayoutNode, startingIndex: number = 0) {
    if (!parent.children) return;
    const idx = parent.children.indexOf(childToRemove, startingIndex);
    if (idx === -1) return;
    parent.children?.splice(idx, 1);
}

/**
 * Finds the node with the given id.
 * @param node The node to search in.
 * @param id The id to search for.
 * @returns The node with the given id or undefined if no node with the given id was found.
 */
export function findNode(node: LayoutNode, id: string): LayoutNode | undefined {
    if (!node) return;
    if (node.id === id) return node;
    if (!node.children) return;
    for (const child of node.children) {
        const result = findNode(child, id);
        if (result) return result;
    }
    return;
}

/**
 * Finds the node with the given blockId by searching the tree.
 * This is useful for finding orphaned blocks that may not be in the leafs array.
 * @param node The node to start the search from.
 * @param blockId The blockId to search for.
 * @returns The node with the given blockId or undefined if not found.
 */
export function findNodeByBlockId(node: LayoutNode, blockId: string): LayoutNode | undefined {
    if (!node) return;
    // In-pane tabs: `blockId` may be a dormant (non-active) member of a
    // stacked leaf, not just its currently-active one.
    if (node.data?.blockId === blockId || node.data?.blockStack?.includes(blockId)) return node;
    if (!node.children) return;
    for (const child of node.children) {
        const result = findNodeByBlockId(child, blockId);
        if (result) return result;
    }
    return;
}

/**
 * Finds the node whose children contains the node with the given id.
 * @param node The node to start the search from.
 * @param id The id to search for.
 * @returns The parent node, or undefined if no node with the given id was found.
 */
export function findParent(node: LayoutNode, id: string): LayoutNode | undefined {
    if (node.id === id || !node.children) return;
    for (const child of node.children) {
        if (child.id === id) return node;
        const retVal = findParent(child, id);
        if (retVal) return retVal;
    }
    return;
}

/**
 * Determines whether a node is valid.
 * @param node The node to validate.
 * @returns True if the node is valid, false otherwise.
 */
function validateNode(node: LayoutNode): boolean {
    if (!node.children == !node.data) {
        console.error("Either children or data must be defined for node, not both");
        return false;
    }

    if (node.children?.length === 0) {
        console.error("Node cannot define an empty array of children");
        return false;
    }
    return true;
}

/**
 * Recursively walk the layout tree starting at the specified node. Run the specified callbacks, if any.
 * @param node The node from which to start the walk.
 * @param beforeWalkCallback An optional callback to run before walking a node's children.
 * @param afterWalkCallback An optional callback to run after walking a node's children.
 */
export function walkNodes(
    node: LayoutNode,
    beforeWalkCallback?: (node: LayoutNode) => void,
    afterWalkCallback?: (node: LayoutNode) => void
) {
    if (!node) return;
    beforeWalkCallback?.(node);
    node.children?.forEach((child) => walkNodes(child, beforeWalkCallback, afterWalkCallback));
    afterWalkCallback?.(node);
}

/**
 * Recursively corrects the tree to minimize nested single-child nodes, remove invalid nodes, and correct invalid flex direction order.
 * @param node The node to start the balancing from.
 * @param beforeWalkCallback Any optional callback to run before walking a node's children.
 * @param afterWalkCallback An optional callback to run after walking a node's children.
 * @returns The corrected node.
 */
export function balanceNode(
    node: LayoutNode,
    beforeWalkCallback?: (node: LayoutNode) => void,
    afterWalkCallback?: (node: LayoutNode) => void
): LayoutNode {
    walkNodes(
        node,
        (node) => {
            if (!validateNode(node)) throw new Error("Invalid node");
            node.children = node.children?.flatMap((child) => {
                // A dissolved column (`columnDissolve` set) must keep the
                // flexDirection it had when its leaf children were minimized —
                // that's what stacks them vertically inside the header strip.
                // It's nested under a sibling column of the same direction by
                // design (see _dissolveColumn), so the alternation rule below
                // would otherwise flip it and lay the headers out sideways.
                if (child.flexDirection === node.flexDirection && child.columnDissolve === undefined) {
                    child.flexDirection = reverseFlexDirection(node.flexDirection);
                }
                if (child.children?.length == 1 && child.children[0].children && !child._slipAnchor) {
                    return child.children[0].children;
                }
                if (child.children?.length === 0) return;
                return child;
            });
            beforeWalkCallback?.(node);
        },
        (node) => {
            node.children = node.children?.filter((v) => v);
            if (node.children?.length === 1 && !node.children[0].children) {
                node.data = node.children[0].data;
                node.id = node.children[0].id;
                node.children = undefined;
            }
            afterWalkCallback?.(node);
        }
    );
    return node;
}

/**
 * Finds the first node in the tree where a new node can be inserted.
 *
 * This will attempt to fill each node until it has maxChildren children. If a node is full, it will move to its children and
 * fill each of them until it has maxChildren children. It will ensure that each child fills evenly before moving to the next
 * layer down.
 *
 * @param node The node to start the search from.
 * @param maxChildren The maximum number of children a node can have.
 * @returns The node to insert into and the index at which to insert.
 */
export function findNextInsertLocation(
    node: LayoutNode,
    maxChildren = DEFAULT_MAX_CHILDREN
): { node: LayoutNode; index: number } {
    const insertLoc = findNextInsertLocationHelper(node, maxChildren, 1);
    return { node: insertLoc?.node, index: insertLoc?.index };
}

/**
 * Traverse the layout tree using the supplied index array to find the node to insert at.
 * @param node The node to start the search from.
 * @param indexArr The array of indices to aid in the traversal.
 * @returns The node to insert into and the index at which to insert.
 */
export function findInsertLocationFromIndexArr(
    node: LayoutNode,
    indexArr: number[]
): { node: LayoutNode; index: number } {
    function normalizeIndex(index: number) {
        const childrenLength = node.children?.length ?? 1;
        const lastChildIndex = childrenLength - 1;
        if (index < 0) {
            return childrenLength - Math.max(index, -childrenLength);
        }
        return Math.min(index, lastChildIndex);
    }
    if (indexArr.length == 0) {
        return;
    }
    const nextIndex = normalizeIndex(indexArr.shift());
    if (indexArr.length == 0 || !node.children) {
        return { node, index: nextIndex };
    }
    return findInsertLocationFromIndexArr(node.children[nextIndex], indexArr);
}

function findNextInsertLocationHelper(
    node: LayoutNode,
    maxChildren: number,
    curDepth: number = 1
): { node: LayoutNode; index: number; depth: number } {
    if (!node) return;
    // A minimized leaf or fully-collapsed branch is locked — never a blind-
    // insert target. Without this, `addChildAt` promoting a minimized leaf
    // into a branch (via `addIntermediateNode`) left the STALE `minimized`
    // flag on the new branch (a leaf-only field, violating the layout
    // doctor's I2) while the inherited-id child silently lost it — the
    // "expanded pane shows a Restore button" corruption (2026-07-18). For a
    // fully-minimized branch this also prevents a blind insert from
    // silently un-collapsing it (every child minimized → one child isn't
    // anymore, with no user action on that branch at all).
    if (isEffectivelyMinimized(node)) return;
    if (!node.children) return { node, index: 1, depth: curDepth };
    let insertLocs: { node: LayoutNode; index: number; depth: number }[] = [];
    if (node.children.length < maxChildren) {
        insertLocs.push({ node, index: node.children.length, depth: curDepth });
    }
    for (const child of node.children.slice().reverse()) {
        insertLocs.push(findNextInsertLocationHelper(child, maxChildren, curDepth + 1));
    }
    insertLocs = insertLocs
        .filter((a) => a)
        .sort((a, b) => Math.pow(a.depth, a.index + maxChildren) - Math.pow(b.depth, b.index + maxChildren));
    return insertLocs[0];
}
