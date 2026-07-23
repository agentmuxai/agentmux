// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Block component model registry — split out of global.ts (see global.ts's
// "Block component model registry" section for the original context).
// Re-exported from global.ts for backward-compat (97 files import from that
// module).

import { getLayoutModelForStaticTab } from "@/layout/index";
import { cleanupBlockAtomCache } from "./block-atom-cache";
import { createBlock } from "./block-layout-actions";

const blockComponentModelMap = new Map<string, BlockComponentModel>();

export function registerBlockComponentModel(blockId: string, bcm: BlockComponentModel) {
    blockComponentModelMap.set(blockId, bcm);
}

export function unregisterBlockComponentModel(blockId: string, owner?: BlockComponentModel) {
    // Owner-checked delete (SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR §3.4):
    // every tab stays mounted, so a block can be transiently mounted twice
    // (e.g. a dangling layout leaf during a cross-tab move). Registration is
    // last-writer-wins; without this check the FIRST mount's unmount deletes
    // the SECOND mount's live registration and tears down its atom cache —
    // leaving the surviving pane unreachable by focus routing (the
    // "non-responsive tab"). Callers pass the exact bcm they registered; the
    // delete only proceeds if that bcm still owns the key.
    if (owner !== undefined && blockComponentModelMap.get(blockId) !== owner) {
        return;
    }
    blockComponentModelMap.delete(blockId);
    cleanupBlockAtomCache(blockId);
}

export function getBlockComponentModel(blockId: string): BlockComponentModel {
    return blockComponentModelMap.get(blockId);
}

export function getAllBlockComponentModels(): BlockComponentModel[] {
    return Array.from(blockComponentModelMap.values());
}

export function getFocusedBlockId(): string {
    const layoutModel = getLayoutModelForStaticTab();
    const focusedLayoutNode = layoutModel.focusedNode();
    return focusedLayoutNode?.data?.blockId;
}

export function refocusNode(blockId: string) {
    if (blockId == null) {
        blockId = getFocusedBlockId();
        if (blockId == null) return;
    }
    const layoutModel = getLayoutModelForStaticTab();
    const layoutNodeId = layoutModel.getNodeByBlockId(blockId);
    if (layoutNodeId?.id == null) return;
    layoutModel.focusNode(layoutNodeId.id);
    const bcm = getBlockComponentModel(blockId);
    const ok = bcm?.viewModel?.giveFocus?.();
    if (!ok) {
        const inputElem = document.getElementById(`${blockId}-dummy-focus`);
        inputElem?.focus();
    }
}

/**
 * Open or focus a pane by view type.
 * If a block with the given viewType already exists in the current tab's layout,
 * focus it. Otherwise create a new block using blockDef (defaults to `{ meta: { view: viewType } }`).
 */
export async function openOrFocusPaneByView(viewType: string, blockDef?: BlockDef): Promise<void> {
    const layoutModel = getLayoutModelForStaticTab();
    for (const bcm of blockComponentModelMap.values()) {
        if (bcm.viewModel?.viewType === viewType) {
            const blockId = (bcm.viewModel as any).blockId as string | undefined;
            if (blockId) {
                const node = layoutModel.getNodeByBlockId(blockId);
                if (node?.id != null) {
                    // Block is in the active tab — focus it.
                    layoutModel.focusNode(node.id);
                    bcm.viewModel.giveFocus?.();
                    return;
                }
                // Block exists on another tab; fall through and open a fresh one here.
            }
        }
    }
    await createBlock(blockDef ?? { meta: { view: viewType } });
}
