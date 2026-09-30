// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A whole-pane (tile) drag's shared state: what its start, the source's
 * release and the end-of-drag cleanup each set, on the drag session.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.1.
 */

import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { beginDrag, endDrag, markReleased, session, type DragEndReason } from "@/app/drag/drag-session";
import { clearCrossTabDrop } from "./crossTabDrag";
import type { LayoutModel } from "./layoutModel";
import { dragState } from "./tilelayout-drag-state";
import type { LayoutNode } from "./types";

export function startTileDrag(node: LayoutNode, model: LayoutModel): void {
    const tabId = model.tabAtom()?.oid;
    dragState.nodeId = node.id;
    dragState.layoutModel = model;
    dragState.node = node;
    beginDrag("tile", { nodeId: node.id, tabId });
    clearCrossTabDrop();
    model.activeDrag._set(true);
    setCurrentDragPayload({ kind: "tile", node, sourceTabId: tabId });
}

/**
 * The source's onDrop. pragmatic fires it for every release, inside the window
 * or not, so the payload stays for the cross-window monitor and the session is
 * only marked released; the end-of-drag cleanup ends it.
 */
export function releaseTileDrag(model: LayoutModel): void {
    dragState.nodeId = null;
    dragState.layoutModel = null;
    dragState.node = null;
    model.activeDrag._set(false);
    markReleased();
}

export function endTileDrag(reason: DragEndReason): void {
    if (session()?.kind === "tile") endDrag(reason);
}
