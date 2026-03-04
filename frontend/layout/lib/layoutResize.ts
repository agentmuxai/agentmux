// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { debounce } from "throttle-debounce";
import { findNode } from "./layoutNode";
import {
    FlexDirection,
    LayoutTreeActionType,
    LayoutTreeResizeNodeAction,
    LayoutTreeSetPendingAction,
    ResizeHandleProps,
} from "./types";
import type { LayoutModel } from "./layoutModel";

export interface ResizeContext {
    handleId: string;
    pixelToSizeRatio: number;
    displayContainerRect?: Dimensions;
    resizeHandleStartPx: number;
    beforeNodeId: string;
    beforeNodeStartSize: number;
    afterNodeId: string;
    afterNodeStartSize: number;
}

export const DefaultGapSizePx = 3;
export const MinNodeSizePx = 40;

/**
 * Callback that is invoked when the TileLayout container is being resized.
 */
export function onContainerResize(model: LayoutModel) {
    model.updateTree();
    model.setter(model.isContainerResizing, true);
    model.stopContainerResizing();
}

/**
 * Create a debounced function to restore animations once the TileLayout container is no longer being resized.
 */
export function createStopContainerResizing(model: LayoutModel) {
    return debounce(30, () => {
        model.setter(model.isContainerResizing, false);
    });
}

/**
 * Callback to update pending node sizes when a resize handle is dragged.
 * @param model The LayoutModel instance.
 * @param resizeHandle The resize handle that is being dragged.
 * @param x The X coordinate of the pointer device, in CSS pixels.
 * @param y The Y coordinate of the pointer device, in CSS pixels.
 */
export function onResizeMove(model: LayoutModel, resizeHandle: ResizeHandleProps, x: number, y: number) {
    const parentIsRow = resizeHandle.flexDirection === FlexDirection.Row;

    // If the resize context is out of date, update it and save it for future events.
    if (model.resizeContext?.handleId !== resizeHandle.id) {
        const parentNode = findNode(model.treeState.rootNode, resizeHandle.parentNodeId);
        const beforeNode = parentNode.children![resizeHandle.parentIndex];
        const afterNode = parentNode.children![resizeHandle.parentIndex + 1];

        const addlProps = model.getter(model.additionalProps);
        const pixelToSizeRatio = addlProps[resizeHandle.parentNodeId]?.pixelToSizeRatio;
        if (beforeNode && afterNode && pixelToSizeRatio) {
            model.resizeContext = {
                handleId: resizeHandle.id,
                displayContainerRect: model.displayContainerRef.current?.getBoundingClientRect(),
                resizeHandleStartPx: resizeHandle.centerPx,
                beforeNodeId: beforeNode.id,
                afterNodeId: afterNode.id,
                beforeNodeStartSize: beforeNode.size,
                afterNodeStartSize: afterNode.size,
                pixelToSizeRatio,
            };
        } else {
            console.error(
                "Invalid resize handle, cannot get the additional properties for the nodes in the resize handle properties."
            );
            return;
        }
    }

    const clientPoint = parentIsRow
        ? x - model.resizeContext.displayContainerRect?.left
        : y - model.resizeContext.displayContainerRect?.top;
    const clientDiff = (model.resizeContext.resizeHandleStartPx - clientPoint) * model.resizeContext.pixelToSizeRatio;
    const minNodeSize = MinNodeSizePx * model.resizeContext.pixelToSizeRatio;
    const beforeNodeSize = model.resizeContext.beforeNodeStartSize - clientDiff;
    const afterNodeSize = model.resizeContext.afterNodeStartSize + clientDiff;

    // If either node will be too small after this resize, don't let it happen.
    if (beforeNodeSize < minNodeSize || afterNodeSize < minNodeSize) {
        return;
    }

    const resizeAction: LayoutTreeResizeNodeAction = {
        type: LayoutTreeActionType.ResizeNode,
        resizeOperations: [
            {
                nodeId: model.resizeContext.beforeNodeId,
                size: beforeNodeSize,
            },
            {
                nodeId: model.resizeContext.afterNodeId,
                size: afterNodeSize,
            },
        ],
    };
    const setPendingAction: LayoutTreeSetPendingAction = {
        type: LayoutTreeActionType.SetPendingAction,
        action: resizeAction,
    };

    model.treeReducer(setPendingAction);
    model.updateTree(false);
}

/**
 * Callback to end the current resize operation and commit its pending action.
 */
export function onResizeEnd(model: LayoutModel) {
    if (model.resizeContext) {
        model.resizeContext = undefined;
        model.treeReducer({ type: LayoutTreeActionType.CommitPendingAction });
    }
}
