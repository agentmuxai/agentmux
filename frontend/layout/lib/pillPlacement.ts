// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A single pane tab placed as a pane of its own, shown as a whole pane's drop is: the layout reflows live to
 * what the release leaves (liveReflow.ts) and a landing zone marks the new pane's place.
 *
 * A tab pill's drag is taken by the panes it is over (pane-tab-foreign-drop.ts). That works while nothing
 * moves, and stops working the moment a drop is shown: the reflow moves the panes, so the pane under the
 * pointer is no longer the one the pointer chose, and the landing zone has no pane under it at all. So the
 * first sample over a pane's placing zone hands the drag to the layout (`beginPillPlacement`): the catcher
 * (PillCatcher.tsx) covers the layout from then on, and each sample is read against the layout's own rects,
 * which no drag changes, as a pane drag's is (placementPreview.ts). The drag goes back to the panes when the
 * pointer comes to where the tab would join a pane instead (a header, the middle of a body), and for the
 * moment the panes take to glide home only the pane that stands there in the layout reads it (`paneReadsPill`).
 *
 * What is shown is what the release makes. A tab that leaves other tabs behind splits the hovered pane in
 * half (`splitOutStackMember`), so that split is the preview; a pane's only tab moves the pane, so the
 * preview is that move. The release commits the drop that is shown, or the one under the pointer when none
 * was shown yet (a quick drag).
 */

import { batch } from "solid-js";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { onSessionEnded } from "@/app/drag/drag-session";
import { findNodeByBlockId } from "./layoutNode";
import type { LayoutModel } from "./layoutModel";
import { effectiveStack } from "./layoutStack";
import { reflowSettleMs } from "./liveReflow";
import { pillStandIn, pillZone, placePill, type PillZone } from "./paneBodyDrop";
import { currentPillDrag, pillDragIn, pillReflowIn, setPillDrag, type PillDrag } from "./pillDragState";
import { clearPlacement, previewPlacement, resetDropIntent, sideOfDirection } from "./placementPreview";

type PlaceZone = Extract<PillZone, { kind: "place" }>;

/** What the layout needs of a pane to read a pointer over it, given by the pane's own tab strip. */
export interface PillPane {
    /** The pane's header row: its height is the band where a tab joins the pane. */
    headerEl: HTMLElement;
    /** False for a header-only pane (minimized, collapsed) and while a pane is maximized: no placing zone. */
    hasBody: () => boolean;
    /** Place the tab beside this pane and mark where it landed. */
    place: (blockId: string, zone: PlaceZone) => string | false;
}

const panes = new WeakMap<LayoutModel, Map<string, PillPane>>();

/** A pane's tab strip is mounted: the layout can read a pointer over the pane. Returns the undo. */
export function registerPillPane(model: LayoutModel, nodeId: string, pane: PillPane): () => void {
    const byNode = panes.get(model) ?? new Map<string, PillPane>();
    panes.set(model, byNode);
    byNode.set(nodeId, pane);
    return () => {
        if (byNode.get(nodeId) === pane) byNode.delete(nodeId);
    };
}

interface Hit {
    nodeId: string;
    /** The pointer in client coordinates, brought inside the pane's rect when it was in a gap beside it. */
    pointer: { x: number; y: number };
}

/** The pane the pointer is over by the layout's own rects, or the nearest one from a gap between panes. */
function paneAt(model: LayoutModel, client: { x: number; y: number }): Hit | undefined {
    const container = model.displayContainerRef?.current;
    if (!container) return undefined;
    const box = container.getBoundingClientRect();
    const [x, y] = [client.x - box.x, client.y - box.y];
    let best: { nodeId: string; rect: Dimensions; dist: number } | undefined;
    for (const leaf of model.leafs()) {
        const rect = model.getNodeRectById(leaf.id);
        if (!rect) continue;
        const dx = Math.max(rect.left - x, 0, x - (rect.left + rect.width));
        const dy = Math.max(rect.top - y, 0, y - (rect.top + rect.height));
        const dist = dx * dx + dy * dy;
        if (!best || dist < best.dist) best = { nodeId: leaf.id, rect, dist };
    }
    if (!best) return undefined;
    const r = best.rect;
    return {
        nodeId: best.nodeId,
        pointer: {
            x: box.x + Math.max(r.left, Math.min(r.left + r.width, x)),
            y: box.y + Math.max(r.top, Math.min(r.top + r.height, y)),
        },
    };
}

/**
 * Whether the pane `nodeId` may read a tab pill's pointer at `client` itself. Not while the layout has the
 * drag. And for the moment after a placement ended, while the panes glide home, only the pane that stands
 * under the pointer in the layout: the one on screen there may be another, still on its way.
 */
export function paneReadsPill(model: LayoutModel, nodeId: string, client: { x: number; y: number }): boolean {
    if (pillDragIn(model)) return false;
    if (!pillReflowIn(model)) return true;
    return paneAt(model, client)?.nodeId === nodeId;
}

/** What the tab would do if released at `client`, by the layout's own rects. */
function zoneAt(pill: PillDrag, client: { x: number; y: number }): { hit: Hit; zone: PillZone } | undefined {
    const { model } = pill;
    const hit = paneAt(model, client);
    const container = model.displayContainerRef?.current;
    const rect = hit && model.getNodeRectById(hit.nodeId);
    if (!hit || !container || !rect) return undefined;
    const ownPane = hit.nodeId === pill.sourceNodeId;
    // A pane's only tab has nowhere to go inside its own pane.
    if (ownPane && pill.lone) return { hit, zone: { kind: "none" } };
    const pane = panes.get(model)?.get(hit.nodeId);
    const box = container.getBoundingClientRect();
    // The header band where the pane stands in the layout, not where the reflow shows it now.
    const headerHeight = pane?.headerEl.getBoundingClientRect().height ?? 0;
    const [left, top] = [box.x + rect.left, box.y + rect.top];
    const zone = pillZone({
        model,
        nodeId: hit.nodeId,
        blockId: pill.blockId,
        pointer: hit.pointer,
        headerRect: headerHeight > 0 ? { left, top, right: left + rect.width, bottom: top + headerHeight } : undefined,
        ownPane,
        hasBody: pane?.hasBody() ?? true,
    });
    return { hit, zone };
}

/**
 * The first sample of a single tab over a pane's placing zone: the layout takes the drag from here. False
 * when the tab is not in this layout (it comes from another window tab): the pane shows its own ghost.
 */
export function beginPillPlacement(model: LayoutModel, blockId: string, client: { x: number; y: number }): boolean {
    if (pillDragIn(model)) return true;
    const root = model.treeState.rootNode;
    const source = root && findNodeByBlockId(root, blockId);
    if (!source?.data) return false;
    const lone = effectiveStack(source.data).length <= 1;
    const standIn = pillStandIn(blockId);
    resetDropIntent(model);
    batch(() => {
        setPillDrag({ model, blockId, sourceNodeId: source.id, lone, standIn, draggedId: lone ? source.id : standIn.id });
        model.showOverlay._set(true);
    });
    followPill(model, client);
    return true;
}

/** One sample of the tab's drag over the layout: show the drop it means, or give the drag back to the panes. */
export function followPill(model: LayoutModel, client: { x: number; y: number }): void {
    const pill = pillDragIn(model);
    if (!pill || pill.dropped) return;
    const at = zoneAt(pill, client);
    const container = model.displayContainerRef?.current;
    const rect = at && model.getNodeRectById(at.hit.nodeId);
    if (!at || at.zone.kind !== "place" || !container || !rect) {
        endPillPlacement(model);
        return;
    }
    const box = container.getBoundingClientRect();
    previewPlacement(model, {
        targetId: at.hit.nodeId,
        rect,
        offset: { x: at.hit.pointer.x - box.x, y: at.hit.pointer.y - box.y },
        draggedId: pill.draggedId,
        foreignNode: pill.lone ? undefined : pill.standIn,
        pill: true,
    });
}

/** The placing is over without a drop: take the shown drop down and let the panes have the drag again. */
export function endPillPlacement(model: LayoutModel): void {
    if (!pillDragIn(model)) return;
    batch(() => {
        clearPlacement(model, true);
        // The reflow stays on while the panes glide home.
        setPillDrag(undefined, reflowSettleMs());
        model.showOverlay._set(false);
    });
}

/**
 * The tab was released over the layout: commit the drop that is shown, or the one under the pointer when
 * none was shown yet. On the next task, as a pane's own drop target does it: the move unmounts the dragged
 * pill, the live drag source. The shown drop stays until then, and goes in the same batch as the tree
 * changes, so the panes are already where the commit puts them.
 */
export function dropPill(model: LayoutModel, client: { x: number; y: number }): void {
    const pill = pillDragIn(model);
    if (!pill || pill.dropped) return;
    // Handled in the window: the cross-window monitor must not also tear the tab off (pane-tab-tearoff.ts).
    setCurrentDragPayload(null);
    const under = pill.armed ? undefined : zoneAt(pill, client);
    const drop: { targetId: string; zone: PlaceZone } | undefined = pill.armed
        ? { targetId: pill.armed.targetId, zone: { kind: "place", direction: pill.armed.direction, side: sideOfDirection(pill.armed.direction)! } }
        : under?.zone.kind === "place"
          ? { targetId: under.hit.nodeId, zone: under.zone }
          : undefined;
    if (!drop) {
        endPillPlacement(model);
        return;
    }
    pill.dropped = true;
    setTimeout(() => {
        batch(() => {
            const pane = panes.get(model)?.get(drop.targetId);
            if (pane) pane.place(pill.blockId, drop.zone);
            else placePill(model, pill.blockId, drop.targetId, drop.zone);
            clearPlacement(model, true);
            setPillDrag(undefined);
            model.showOverlay._set(false);
        });
    }, 0);
}

// The drag ended somewhere else (cancelled, or released outside the layout): nothing stays shown.
onSessionEnded(() => {
    const pill = currentPillDrag();
    if (pill && !pill.dropped) endPillPlacement(pill.model);
});
