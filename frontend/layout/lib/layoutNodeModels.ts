// Copyright 2025, Command Line Inc.
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
    const blockId = node.data.blockId;
    const addlPropsAtom = getNodeAdditionalPropertiesAtom(model, nodeid);
    if (!model.nodeModels.has(nodeid)) {
        model.nodeModels.set(nodeid, {
            additionalProps: addlPropsAtom,
            innerRect: createMemo(() => {
                const addlProps = addlPropsAtom();
                const numLeafs = model.numLeafs();
                const gapSizePx = model.gapSizePx();
                if (numLeafs > 1 && addlProps?.rect) {
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
            isMagnified: createMemo(() => {
                const treeState = model.localTreeStateAtom();
                return treeState.magnifiedNodeId === nodeid;
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
            focusNode: () => model.focusNode(nodeid),
            dragHandleRef: { current: null as HTMLDivElement | null },
            displayContainerRef: model.displayContainerRef,
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
        if (leaf.data.blockId === blockId) {
            return leaf;
        }
    }
    return null;
}

/**
 * Get a signal accessor containing the additional properties associated with a given node.
 * @param model The LayoutModel instance.
 * @param nodeId The ID of the node for which to retrieve the additional properties.
 * @returns A signal accessor containing the additional properties associated with the given node.
 */
export function getNodeAdditionalPropertiesAtom(model: LayoutModel, nodeId: string): () => LayoutNodeAdditionalProps {
    return createMemo(() => {
        const addlProps = model.additionalProps();
        if (addlProps.hasOwnProperty(nodeId)) return addlProps[nodeId];
        return undefined;
    });
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
