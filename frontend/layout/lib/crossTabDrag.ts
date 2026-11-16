// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Cross-tab pane drag state + commit (SPEC_PANE_DRAG_TO_TAB_2026_07_10.md).
//
// A pane dragged over a different tab's button spring-switches the UI to
// that tab (droppable-tab.tsx); the target tab's own TileLayout overlay then
// previews the landing slot via the normal ComputeMove/placeholder path
// (with the foreign node passed via LayoutTreeComputeMoveNodeAction.nodeToMove).
// On release, the pending Move must NOT commit locally — the dragged block
// still belongs to the source tab in the backend — so TileLayout routes the
// drop here instead, and RedockFloatingPane performs the sanctioned
// MoveBlock + queue_target_layout_split + queue_source_layout_delete flow
// (the same one cross-window redocks use; its saga only rejects
// source === target TAB, so same-workspace calls are already supported).

import { atoms } from "@/store/global";
import * as MOS from "@/app/store/mos";
import { redockFloaterPane } from "@/app/workspace/floater-panes";
import { Logger } from "@/util/logger";
import { fireAndForget } from "@/util/util";
import { holdDrop, releaseHeldDrop } from "./heldDrop";
import { getLayoutModelForTabById } from "./layoutModelHooks";
import { reconcileWithStoredLayout } from "./layoutIntents";
import { DropDirection } from "./types";

/**
 * Clamp Outer* drop directions to their inner equivalents for CROSS-TAB
 * drags. In-tab, Outer* commits a Move that inserts at the grandparent
 * level (spanning the full cross axis) — and the ghost placeholder
 * previews exactly that. But a cross-tab drop commits through
 * RedockFloatingPane → queue_target_layout_split, which can only express
 * "split THIS leaf" (Outer* becomes a 20% band of the target leaf) — a
 * visibly different result from what the ghost showed. Clamping to the
 * inner direction makes preview and commit agree: both are a half-split
 * of the hovered leaf.
 */
export function clampCrossTabDirection(dir: DropDirection | undefined): DropDirection | undefined {
    if (dir === undefined) return undefined;
    if (dir >= DropDirection.OuterTop && dir <= DropDirection.OuterLeft) {
        return (dir - 4) as DropDirection;
    }
    return dir;
}

export type CrossTabDropRecord = {
    /** The tabs of the dragged pane, in order (one for a lone pane or a single pill), the first landing as the pane. */
    blockIds: string[];
    /** The visible one of `blockIds`. */
    activeBlockId: string;
    sourceTabId: string;
    targetTabId: string;
    targetBlockId: string;
    direction: DropDirection;
};

// The most recent cross-tab hover state, captured during the target
// overlay's onDrag (while TileLayout's module-level drag globals are
// guaranteed alive — pragmatic-dnd's per-callback ordering at drop time is
// not part of its contract, so drop handlers must NOT read those globals).
// Mirrors what the ghost placeholder is showing; consumed exactly once at
// drop time. When null at drop (e.g. drop straight onto a tab button with
// no in-layout hover), the redock falls back to a plain append.
let crossTabDrop: CrossTabDropRecord | null = null;

export function noteCrossTabDrop(record: CrossTabDropRecord | null): void {
    crossTabDrop = record;
}

export function clearCrossTabDrop(): void {
    crossTabDrop = null;
}

/**
 * Returns and clears the pending record if it targets `targetTabId` —
 * a drop consumes it exactly once, and only in the tab it was computed for.
 */
export function takeCrossTabDropFor(targetTabId: string | undefined): CrossTabDropRecord | null {
    if (!crossTabDrop || !targetTabId || crossTabDrop.targetTabId !== targetTabId) return null;
    const record = crossTabDrop;
    crossTabDrop = null;
    return record;
}

/** The tab of this window's workspace that holds `blockId`, when one does. */
export function tabOfBlock(blockId: string): string | undefined {
    // The workspace lists its pinned window tabs apart from the others. Read without them, a pane tab dragged
    // out of a pinned window tab had no tab, and its drop on another window tab moved nothing.
    const ws = atoms.workspace();
    const tabIds = [...(ws?.tabids ?? []), ...(ws?.pinnedtabids ?? [])];
    return tabIds.find((tabId) => MOS.getObjectValue<Tab>(MOS.makeORef("tab", tabId))?.blockids?.includes(blockId));
}

/**
 * Move `blockIds` (a whole pane's tabs, or one pill) from `sourceTabId` into `targetTabId`: the one
 * path every drop on another window tab takes, a pane or a pill, on the tab's button or in its layout
 * after the spring switch. The first block lands as the pane, at the ghost's slot when `targetBlockId`
 * and `direction` name one, or as a tab of `targetBlockId`'s pane with `asTab`; the rest follow it in
 * order, and `activeBlockId` ends visible. Fire-and-forget: the layout updates arrive via each tab's
 * pendingbackendactions, so there is nothing to await here.
 *
 * Where the target tab's layout shows the drop (a release in it, after the spring switch), what it shows
 * stays shown until the pane arrives (heldDrop.ts), for every caller: the pane's own release and a tab
 * pill's alike.
 */
export function moveBlocksToTab(opts: {
    blockIds: readonly string[];
    activeBlockId?: string;
    sourceTabId: string;
    targetTabId: string;
    targetBlockId?: string | null;
    direction?: DropDirection | null;
    asTab?: boolean;
}): void {
    const wsId = atoms.workspace()?.oid;
    const [lead] = opts.blockIds;
    if (!wsId || !lead || !opts.sourceTabId || !opts.targetTabId || opts.sourceTabId === opts.targetTabId) {
        Logger.warn("dnd", "cross-tab move skipped — missing/invalid args", { ...opts, wsId });
        return;
    }
    Logger.info("dnd", "cross-tab move", { ...opts, wsId });
    const pane = { blockId: lead, members: [...opts.blockIds], activeBlockId: opts.activeBlockId ?? lead };
    const ghost = { blockId: opts.targetBlockId ?? null, dir: opts.direction ?? null, asTab: opts.asTab };
    // A tab that joins a pane's tabs moves no pane: there is no drop of the layout's to hold.
    const target = opts.asTab || opts.direction == null ? undefined : getLayoutModelForTabById(opts.targetTabId);
    if (target) holdDrop(target, lead);
    fireAndForget(async () => {
        try {
            await redockFloaterPane(pane, { tabId: opts.sourceTabId, workspaceId: wsId }, { tabId: opts.targetTabId, workspaceId: wsId }, ghost);
            // Do NOT delete the source leaf here: this runs at RPC
            // completion, which can precede the drag's dragend on a slow
            // frame — and unmounting the drag source before dragend
            // suppresses the event entirely, wedging pragmatic's teardown
            // (the dead-source-tab bug). Source-leaf removal is owned by
            // srv, which takes the block out of the source tab's tree and
            // queues the matching delete for this window.
        } catch (e) {
            Logger.error("dnd", "cross-tab move failed", { error: String(e) });
            // No pane is coming: the drop that was shown for it goes.
            if (target) releaseHeldDrop(target);
            // The characteristic failure here is "block X is in tab Y,
            // not <sourceTabId>": the source tab showed a leaf srv doesn't
            // hold. Show the source tab as srv stores it; if the drag hasn't
            // ended yet, that runs when it does (`layoutDragEnded`).
            const sourceModel = getLayoutModelForTabById(opts.sourceTabId);
            if (sourceModel) reconcileWithStoredLayout(sourceModel);
        }
    });
}
