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

// Codex P1/P2 on PR #3187: `pane-leaf-chrome.tsx`'s keep-alive terminal tabs
// (SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md's keep-alive
// follow-up) mount EVERY stack member's `<Block>` simultaneously, not just
// the active one — each still registers itself here under its own blockId,
// same as always. Every existing consumer of the two "all panes" functions
// below (`TermViewModel.multiInputHandler`'s keystroke broadcast,
// `zoom.ts`'s all-panes stepper, `keymodel.ts`'s multi-input-eligible
// terminal count) assumes one registry entry == one currently-visible pane
// — true before keep-alive (a dormant stack member was simply unmounted and
// therefore never registered at all), no longer true now that a hidden
// terminal tab stays mounted AND registered. Filtering here, once, fixes
// every current (and future) "all panes" consumer instead of requiring each
// call site to remember its own filter.
//
// `node.data.blockId` (`layoutStack.ts`'s mutators) is reassigned to
// whichever stack member is ACTIVE on every switch — `getNodeByBlockId`
// resolves the leaf for ANY member, dormant or active, of a stacked leaf
// (`leaf.data.blockStack?.includes(blockId)`), so comparing its return
// value's OWN `.blockId` against the id being tested is what actually
// distinguishes "the currently active/visible member" from "some other
// blockId that merely still exists somewhere in this leaf's stack." A
// no-op for every non-keep-alive pane type: those only ever have their
// current active member registered at all, so this always resolves true
// for whatever's actually in the map.
function isActiveStackMember(blockId: string): boolean {
    const layoutModel = getLayoutModelForStaticTab();
    const node = layoutModel.getNodeByBlockId(blockId);
    return node?.data?.blockId === blockId;
}

export function getAllBlockComponentModels(): BlockComponentModel[] {
    return Array.from(blockComponentModelMap.keys())
        .filter(isActiveStackMember)
        .map((blockId) => blockComponentModelMap.get(blockId));
}

// `ViewModel`'s base interface does not declare `blockId` (concrete view
// models each carry it as their own implementation detail, not a common
// interface field), so a caller that needs BOTH the id and the model — e.g.
// zoom.ts's all-panes stepper — cannot recover it from a plain
// getAllBlockComponentModels() value. The map is already keyed on blockId;
// this just exposes that key alongside its value instead of discarding it.
export function getAllBlockComponentModelEntries(): [string, BlockComponentModel][] {
    return Array.from(blockComponentModelMap.entries()).filter(([blockId]) => isActiveStackMember(blockId));
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
