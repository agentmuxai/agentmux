// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Block creation / layout actions — split out of global.ts (see global.ts's
// "Block creation / layout actions" section for the original context).
// Re-exported from global.ts for backward-compat (97 files import from that
// module).

import {
    getLayoutModelForStaticTab,
    LayoutTreeActionType,
    LayoutTreeInsertNodeAction,
    markBlockRecentlyCreated,
    newLayoutNode,
} from "@/layout/index";
import {
    LayoutTreeReplaceNodeAction,
    LayoutTreeSplitHorizontalAction,
    LayoutTreeSplitVerticalAction,
} from "@/layout/lib/types";
import { fireAndForget } from "@/util/util";
import { ObjectService } from "./services";

export async function createBlockSplitHorizontally(
    blockDef: BlockDef,
    targetBlockId: string,
    position: "before" | "after"
): Promise<string> {
    const layoutModel = getLayoutModelForStaticTab();
    const rtOpts: RuntimeOpts = { termsize: { rows: 25, cols: 80 } };
    const newBlockId = await ObjectService.CreateBlock(blockDef, rtOpts);
    markBlockRecentlyCreated(newBlockId);
    const targetNodeId = layoutModel.getNodeByBlockId(targetBlockId)?.id;
    if (targetNodeId == null) throw new Error(`targetNodeId not found for blockId: ${targetBlockId}`);
    const splitAction: LayoutTreeSplitHorizontalAction = {
        type: LayoutTreeActionType.SplitHorizontal,
        targetNodeId,
        newNode: newLayoutNode(undefined, undefined, undefined, { blockId: newBlockId }),
        position,
        focused: true,
    };
    layoutModel.treeReducer(splitAction);
    return newBlockId;
}

export async function createBlockSplitVertically(
    blockDef: BlockDef,
    targetBlockId: string,
    position: "before" | "after"
): Promise<string> {
    const layoutModel = getLayoutModelForStaticTab();
    const rtOpts: RuntimeOpts = { termsize: { rows: 25, cols: 80 } };
    const newBlockId = await ObjectService.CreateBlock(blockDef, rtOpts);
    markBlockRecentlyCreated(newBlockId);
    const targetNodeId = layoutModel.getNodeByBlockId(targetBlockId)?.id;
    if (targetNodeId == null) throw new Error(`targetNodeId not found for blockId: ${targetBlockId}`);
    const splitAction: LayoutTreeSplitVerticalAction = {
        type: LayoutTreeActionType.SplitVertical,
        targetNodeId,
        newNode: newLayoutNode(undefined, undefined, undefined, { blockId: newBlockId }),
        position,
        focused: true,
    };
    layoutModel.treeReducer(splitAction);
    return newBlockId;
}

export async function createBlock(blockDef: BlockDef, magnified = false, ephemeral = false): Promise<string> {
    const layoutModel = getLayoutModelForStaticTab();
    const rtOpts: RuntimeOpts = { termsize: { rows: 25, cols: 80 } };
    const blockId = await ObjectService.CreateBlock(blockDef, rtOpts);
    // Mark BEFORE branching — the ephemeral path (below) also inserts this
    // block into the local tree (via addEphemeralNodeToLayout, later) ahead
    // of tab.blockids catching up, same race as the non-ephemeral path.
    markBlockRecentlyCreated(blockId);
    if (ephemeral) {
        layoutModel.newEphemeralNode(blockId);
        return blockId;
    }
    const insertNodeAction: LayoutTreeInsertNodeAction = {
        type: LayoutTreeActionType.InsertNode,
        node: newLayoutNode(undefined, undefined, undefined, { blockId }),
        magnified,
        focused: true,
    };
    layoutModel.treeReducer(insertNodeAction);
    return blockId;
}

export async function replaceBlock(blockId: string, blockDef: BlockDef, focus: boolean): Promise<string> {
    const layoutModel = getLayoutModelForStaticTab();
    const rtOpts: RuntimeOpts = { termsize: { rows: 25, cols: 80 } };
    const newBlockId = await ObjectService.CreateBlock(blockDef, rtOpts);
    markBlockRecentlyCreated(newBlockId);
    setTimeout(() => {
        fireAndForget(() => ObjectService.DeleteBlock(blockId));
    }, 300);
    const targetNodeId = layoutModel.getNodeByBlockId(blockId)?.id;
    if (targetNodeId == null) throw new Error(`targetNodeId not found for blockId: ${blockId}`);
    const replaceNodeAction: LayoutTreeReplaceNodeAction = {
        type: LayoutTreeActionType.ReplaceNode,
        targetNodeId,
        newNode: newLayoutNode(undefined, undefined, undefined, { blockId: newBlockId }),
        focused: focus,
    };
    layoutModel.treeReducer(replaceNodeAction);
    return newBlockId;
}

