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
// terminal tab stays mounted AND registered.
//
// ReAgent P1 on this PR: a first version of this fix filtered via
// `getLayoutModelForStaticTab().getNodeByBlockId(blockId)`, comparing the
// found leaf's own (always-active) `data.blockId` — but
// `getLayoutModelForStaticTab()` only resolves the CURRENTLY ACTIVE TAB's
// own tree (`atoms.activeTabId()`). A block belonging to any OTHER (open
// but not front-most) tab can never be found by that lookup at all, so
// that version silently excluded every pane in every background TAB too —
// far broader than the actual dormant-stack-member scope this exists to
// cover, and a real regression for `stepAllPanes` (which its own spec,
// SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md, defines as
// "every pane in the current window", not just the active tab).
//
// Fixed by tracking dormancy explicitly instead of re-deriving it from a
// tab-scoped layout lookup: `pane-leaf-chrome.tsx`'s keep-alive rendering
// is the ONE place that actually knows "this blockId is a currently-hidden
// stack member," so it marks that directly via `setKeepAliveBlockDormant`
// below — no layout/tab resolution involved at all, so cross-tab panes are
// never touched by this filter. A no-op for every non-keep-alive pane
// type and for a block in any other tab: nothing ever marks those,
// so they're never excluded.
const dormantKeepAliveBlockIds = new Set<string>();

/**
 * Marks `blockId` as a currently-HIDDEN, kept-alive stack member (a
 * terminal tab that isn't the active one but stays mounted+registered
 * anyway) — called only by `pane-leaf-chrome.tsx`'s keep-alive rendering,
 * reactively, as a stack member's own visibility toggles. Clearing this
 * (dormant=false, including on that member's own unmount/tab-close) is
 * just as load-bearing as setting it: a stale `true` entry would silently
 * exclude an ordinary, currently-visible pane from every "all panes"
 * consumer forever.
 */
export function setKeepAliveBlockDormant(blockId: string, dormant: boolean) {
    if (dormant) {
        dormantKeepAliveBlockIds.add(blockId);
    } else {
        dormantKeepAliveBlockIds.delete(blockId);
    }
}

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
    // Safety net alongside pane-leaf-chrome.tsx's own onCleanup-driven
    // clear: a real unregistration means the dormancy marker (if any) can
    // never become meaningful again for this blockId.
    dormantKeepAliveBlockIds.delete(blockId);
    cleanupBlockAtomCache(blockId);
}

export function getBlockComponentModel(blockId: string): BlockComponentModel {
    return blockComponentModelMap.get(blockId);
}

export function getAllBlockComponentModels(): BlockComponentModel[] {
    return Array.from(blockComponentModelMap.entries())
        .filter(([blockId]) => !dormantKeepAliveBlockIds.has(blockId))
        .map(([, bcm]) => bcm);
}

// `ViewModel`'s base interface does not declare `blockId` (concrete view
// models each carry it as their own implementation detail, not a common
// interface field), so a caller that needs BOTH the id and the model — e.g.
// zoom.ts's all-panes stepper — cannot recover it from a plain
// getAllBlockComponentModels() value. The map is already keyed on blockId;
// this just exposes that key alongside its value instead of discarding it.
export function getAllBlockComponentModelEntries(): [string, BlockComponentModel][] {
    return Array.from(blockComponentModelMap.entries()).filter(([blockId]) => !dormantKeepAliveBlockIds.has(blockId));
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
