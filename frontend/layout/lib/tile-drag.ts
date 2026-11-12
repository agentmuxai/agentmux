// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A whole-pane (tile) drag's shared state: what its start, the source's
 * release and the end-of-drag cleanup each set, on the drag session.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.1.
 */

import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { beginDrag, endDrag, markReleased, onSessionEnded, session, type DragEndReason } from "@/app/drag/drag-session";
import { watchEscapeDuringDrag } from "@/app/drag/escape-watch";
import { endedByEscape } from "@/app/drag/native-cancel";
import { paneTearOffOf } from "@/app/drag/pane-tearoff";
import { clearCrossTabDrop } from "./crossTabDrag";
import { layoutDragEnded } from "./layoutIntents";
import type { LayoutModel } from "./layoutModel";
import { landingSiteOf } from "./landingSite";
import { landingRectOf } from "./liveReflow";
import { resetAllDropIntent } from "./placementPreview";
import { leaveWindow, returnToWindow } from "./tearOffPreview";
import { dragState } from "./tilelayout-drag-state";
import type { LayoutNode } from "./types";

/** Stops the window watch of the drag in progress (or of the last one, until its `dragend`). */
let stopWindowWatch: (() => void) | undefined;

/**
 * Show the tear-off in `model`'s layout while the pointer of the drag of `nodeId` is outside the window, where a
 * release tears the pane off (tearOffPreview.ts). A `dragleave` to nowhere is the pointer leaving the window,
 * as pragmatic-dnd reads it; a `dragenter` from nowhere is it coming back. A `dragend` with no drop effect may
 * be Escape: then nothing is torn off. It ends with its own `dragend`, since the source's release comes first.
 */
export function watchWindowExit(model: LayoutModel, nodeId: string): () => void {
    if (typeof document === "undefined") return () => {};
    const left = (e: DragEvent) => {
        if (e.relatedTarget === null) leaveWindow(model, nodeId);
    };
    const entered = (e: DragEvent) => {
        if (e.relatedTarget === null) returnToWindow(model);
    };
    const ended = (e: DragEvent) => {
        stop();
        if (e.dataTransfer?.dropEffect !== "none") return;
        void endedByEscape("none").then((cancelled) => {
            if (cancelled) returnToWindow(model);
        });
    };
    const stop = () => {
        document.removeEventListener("dragleave", left);
        document.removeEventListener("dragenter", entered);
        document.removeEventListener("dragend", ended);
        if (stopWindowWatch === stop) stopWindowWatch = undefined;
    };
    document.addEventListener("dragleave", left);
    document.addEventListener("dragenter", entered);
    document.addEventListener("dragend", ended);
    return stop;
}

export function startTileDrag(node: LayoutNode, model: LayoutModel): void {
    stopWindowWatch?.();
    stopWindowWatch = watchWindowExit(model, node.id);
    const tabId = model.tabAtom()?.oid;
    dragState.nodeId = node.id;
    dragState.layoutModel = model;
    dragState.node = node;
    beginDrag("tile", { nodeId: node.id, tabId });
    const blockId = paneTearOffOf(node)?.blockId;
    // The pane's picture in hand takes the shape of its landing rect while the live reflow shows a drop, and
    // is taken again of the pane standing in that rect once the panes have come to rest there.
    const landing = blockId && {
        rect: () => (model.liveReflowActive() ? landingRectOf(model) : undefined),
        active: () => model.activeDrag(),
        settled: () => model.landingSettled(),
        site: () => landingSiteOf(model, blockId),
    };
    watchEscapeDuringDrag(blockId && landing ? { blockId, landing } : undefined);
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
    resetAllDropIntent();
    layoutDragEnded(model);
    markReleased();
}

export function endTileDrag(reason: DragEndReason): void {
    if (session()?.kind === "tile") endDrag(reason);
}

// A tile session that ended before its source released it (the safety net,
// or a new drag replacing a stranded one): do what the source's onDrop would
// have. A stuck activeDrag freezes every pane of the source window tab.
onSessionEnded(({ session: s }) => {
    if (s.kind !== "tile" || s.released || !dragState.layoutModel) return;
    releaseTileDrag(dragState.layoutModel);
});
