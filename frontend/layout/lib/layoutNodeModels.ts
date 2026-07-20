// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignalAtom, fireAndForget } from "@/util/util";
import type { Properties as CSSProperties } from "csstype";
import { createMemo } from "solid-js";
import { LayoutNode, LayoutNodeAdditionalProps, NodeModel } from "./types";
import type { LayoutModel } from "./layoutModel";

/**
 * Gets the node model for the given node.
 * @param model The LayoutModel instance.
 * @param node The node for which to retrieve the node model.
 * @returns The node model for the given node.
 */
export function getNodeModel(model: LayoutModel, node: LayoutNode): NodeModel {
    const nodeid = node.id;
    // In-pane tabs: a leaf with a block stack renders its ACTIVE member, not
    // necessarily `data.blockId` alone (the two are kept in sync by
    // layoutStack.ts's mutators, but activeBlockId is the field of intent).
    // Absent/no-stack falls back to the legacy field — zero behavior change
    // for every pane that never gets a stack. See layoutStack.ts's header
    // comment for why this is captured once (not reactive) here: switching
    // the active member works via a remount, driven by the tile renderer's
    // key function, not by this value changing under a live NodeModel.
    const blockId = node.data.activeBlockId || node.data.blockId;
    const addlPropsAtom = getNodeAdditionalPropertiesAtom(model, nodeid);
    if (!model.nodeModels.has(nodeid)) {
        // Create memos inside the model's own reactive root so they survive
        // component mount/unmount cycles during tab switches.
        model.runInModelRoot(() => {
            model.nodeModels.set(nodeid, {
                additionalProps: addlPropsAtom,
                innerRect: createMemo(() => {
                    const treeState = model.localTreeStateAtom();
                    // When magnified, return null so content fills the overlay container naturally
                    if (treeState.magnifiedNodeId === nodeid) {
                        return null;
                    }
                    const addlProps = addlPropsAtom();
                    const numLeafs = model.numLeafs();
                    const gapSizePx = model.gapSizePx();
                    if (numLeafs > 1 && addlProps?.rect && addlProps?.transform) {
                        return {
                            width: `${addlProps.transform.width} - ${gapSizePx}px`,
                            height: `${addlProps.transform.height} - ${gapSizePx}px`,
                        } as CSSProperties;
                    } else {
                        return null;
                    }
                }),
                nodeId: nodeid,
                blockId,
                blockNum: createMemo(() => model.leafOrder().findIndex((leafEntry) => leafEntry.nodeid === nodeid) + 1),
                isFocused: createMemo(() => {
                    const treeState = model.localTreeStateAtom();
                    return treeState.focusedNodeId === nodeid;
                }),
                numLeafs: model.numLeafs,
                isResizing: model.isResizing,
                isSplitterDragging: model.isSplitterDragging,
                isMagnified: createMemo(() => {
                    const treeState = model.localTreeStateAtom();
                    return treeState.magnifiedNodeId === nodeid;
                }),
                isMinimized: createMemo(() => {
                    const minimizedIds = model.minimizedNodeIds();
                    return minimizedIds.has(nodeid);
                }),
                canMinimize: createMemo(() => {
                    const minimizedIds = model.minimizedNodeIds();
                    // Restoring is always allowed; minimizing requires at
                    // least one OTHER expanded pane to remain (the window
                    // must never become all-headers).
                    if (minimizedIds.has(nodeid)) return true;
                    return model.numLeafs() - minimizedIds.size > 1;
                }),
                isEphemeral: createMemo(() => {
                    const ephemeralNode = model.ephemeralNode();
                    return ephemeralNode?.id === nodeid;
                }),
                addEphemeralNodeToLayout: () => model.addEphemeralNodeToLayout(),
                animationTimeS: model.animationTimeS,
                ready: model.ready,
                disablePointerEvents: model.activeDrag,
                onClose: () => {
                    fireAndForget(() => model.closeNode(nodeid));
                },
                toggleMagnify: () => model.magnifyNodeToggle(nodeid),
                toggleMinimize: () => model.minimizeNodeToggle(nodeid),
                focusNode: () => model.focusNode(nodeid),
                dragHandleRef: { current: null as HTMLDivElement | null },
                displayContainerRef: model.displayContainerRef,
            });
        });
    }
    const nodeModel = model.nodeModels.get(nodeid);
    return nodeModel;
}

/**
 * Remove orphaned node models when their corresponding leaf is deleted.
 * @param model The LayoutModel instance.
 * @param leafOrder The new leaf order array to use when locating orphaned nodes.
 */
export function cleanupNodeModels(model: LayoutModel, leafOrder: LeafOrderEntry[]) {
    const orphanedNodeModels = [...model.nodeModels.keys()].filter(
        (id) => !leafOrder.find((leafEntry) => leafEntry.nodeid == id)
    );
    for (const id of orphanedNodeModels) {
        model.nodeModels.delete(id);
    }
}

/**
 * Get the layout node matching the specified blockId.
 * @param model The LayoutModel instance.
 * @param blockId The blockId that the returned node should contain.
 * @returns The node containing the specified blockId, null if not found.
 */
export function getNodeByBlockId(model: LayoutModel, blockId: string): LayoutNode {
    for (const leaf of model.leafs()) {
        // In-pane tabs: `blockId` may be a dormant (non-active) member of a
        // stacked leaf, not just its currently-active one.
        if (leaf.data.blockId === blockId || leaf.data.blockStack?.includes(blockId)) {
            return leaf;
        }
    }
    return null;
}

/** The key `<Key each={leafs()} by={...}>` uses to identify a leaf's
 *  rendered subtree in the tile renderer (`TileLayout.{win32,linux,darwin}.tsx`).
 *  Ordinary leaves key on `node.id` alone, unchanged from before block
 *  stacks existed. A stacked leaf's key also incorporates `activeBlockId`,
 *  so switching the active member changes the key — which makes `<Key>`
 *  tear down and remount the leaf's subtree, giving the new active block a
 *  freshly-constructed `NodeModel`/`ViewModel` exactly the way every other
 *  blockId transition in this codebase already works (see layoutStack.ts's
 *  header comment for why a remount, not a reactive update, is correct
 *  here). Zero-cost / zero-behavior-change for every non-stacked leaf. */
export function activeKeyFor(node: LayoutNode): string {
    return node.data?.activeBlockId ? `${node.id}:${node.data.activeBlockId}` : node.id;
}

/**
 * Get a signal accessor containing the additional properties associated with a given node.
 * @param model The LayoutModel instance.
 * @param nodeId The ID of the node for which to retrieve the additional properties.
 * @returns A signal accessor containing the additional properties associated with the given node.
 */
export function getNodeAdditionalPropertiesAtom(model: LayoutModel, nodeId: string): () => LayoutNodeAdditionalProps {
    return model.runInModelRoot(() =>
        createMemo(() => {
            const addlProps = model.additionalProps();
            if (addlProps.hasOwnProperty(nodeId)) return addlProps[nodeId];
            return undefined;
        })
    );
}

/**
 * Get additional properties associated with a given node.
 * @param model The LayoutModel instance.
 * @param nodeId The ID of the node for which to retrieve the additional properties.
 * @returns The additional properties associated with the given node.
 */
export function getNodeAdditionalPropertiesById(model: LayoutModel, nodeId: string): LayoutNodeAdditionalProps {
    const addlProps = model.additionalProps();
    if (addlProps.hasOwnProperty(nodeId)) return addlProps[nodeId];
}

/**
 * Get the CSS transform associated with a given node.
 * @param model The LayoutModel instance.
 * @param nodeId The ID of the node for which to retrieve the CSS transform.
 * @returns The CSS transform associated with the given node.
 */
export function getNodeTransformById(model: LayoutModel, nodeId: string): CSSProperties {
    return getNodeAdditionalPropertiesById(model, nodeId)?.transform;
}

/**
 * Get the computed dimensions in CSS pixels of a given node.
 * @param model The LayoutModel instance.
 * @param nodeId The ID of the node for which to retrieve the computed dimensions.
 * @returns The computed dimensions of the given node, in CSS pixels.
 */
export function getNodeRectById(model: LayoutModel, nodeId: string): Dimensions {
    return getNodeAdditionalPropertiesById(model, nodeId)?.rect;
}
