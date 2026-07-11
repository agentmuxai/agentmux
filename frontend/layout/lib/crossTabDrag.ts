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
import { WorkspaceService } from "@/app/store/services";
import { Logger } from "@/util/logger";
import { fireAndForget } from "@/util/util";
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
    blockId: string;
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

/**
 * Commit a cross-tab pane move. Fire-and-forget: the layout updates arrive
 * via each tab's pendingbackendactions, so there is nothing to await here.
 */
export function redockDraggedPane(opts: {
    blockId: string;
    sourceTabId: string;
    targetTabId: string;
    targetBlockId?: string | null;
    direction?: DropDirection | null;
}): void {
    const wsId = atoms.workspace()?.oid;
    if (!wsId || !opts.blockId || !opts.sourceTabId || !opts.targetTabId || opts.sourceTabId === opts.targetTabId) {
        Logger.warn("dnd", "cross-tab redock skipped — missing/invalid args", { ...opts, wsId });
        return;
    }
    Logger.info("dnd", "cross-tab pane redock", { ...opts, wsId });
    fireAndForget(async () => {
        try {
            await WorkspaceService.RedockFloatingPane(
                opts.blockId,
                opts.sourceTabId,
                wsId,
                opts.targetTabId,
                wsId,
                opts.targetBlockId ?? null,
                opts.direction ?? null,
            );
        } catch (e) {
            Logger.error("dnd", "cross-tab pane redock failed", { error: String(e) });
        }
    });
}
