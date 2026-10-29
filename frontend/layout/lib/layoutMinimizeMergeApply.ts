// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The doing half of the merge on minimize (the choice of target is layoutMinimizeMerge.ts). Kept
 * apart because it needs the stack mutators, whose imports (RPC, services) the layout's pure
 * modules must not pull in; layout/index.ts registers it with `setMinimizeMerger`.
 */

import { LANDING_BOUNCE_MS, markLanded } from "@/app/element/pane-tab-landing";
import { DURATION_MODERATE_MS, EASE_EMPHASIZED } from "@/app/motion/tokens";
import { shouldReduceMotion } from "@/app/motion/motion";
import { revealInStrip, STRIP_EDGE_FADE_PX } from "@/app/element/strip-reveal";
import { findNode, findNodeByBlockId } from "./layoutNode";
import { findMinimizeMergeTarget } from "./layoutMinimizeMerge";
import type { LayoutModel } from "./layoutModel";
import { effectiveStack, moveBlocksInStack } from "./layoutStack";

/**
 * Merge the pane `nodeId` into its minimized neighbour instead of minimizing it: all its tabs are
 * appended to the end of the target's strip, in their strip order, and the emptied pane is removed
 * as a move (its space goes to the expanded panes, as for a minimize). Returns false, touching
 * nothing, when there is no target or the move could not be applied, so the caller minimizes as before.
 */
export function mergeIntoMinimizedNeighbour(model: LayoutModel, nodeId: string): boolean {
    const root = model.treeState.rootNode;
    const target = findMinimizeMergeTarget(root, nodeId);
    const node = root && findNode(root, nodeId);
    if (!target?.data || !node?.data) return false;
    const targetBlockId = target.data.blockId;
    const blockIds = effectiveStack(node.data);
    const reduceMotion = shouldReduceMotion("transition");
    const from = reduceMotion ? undefined : pillRects(node.data.blockId, blockIds);

    // A maximized pane that merges ends its maximize, as a minimize does (removeLeafEmptiedByMove).
    const moved = moveBlocksInStack(model, blockIds, targetBlockId);
    if (!moved) return false;

    if (moved.length) {
        const landedIn = model.treeState.rootNode && findNodeByBlockId(model.treeState.rootNode, targetBlockId);
        if (landedIn && !reduceMotion) {
            markLanded(moved, landedIn.id);
            if (from) travelPills(targetBlockId, moved, from);
        }
        revealArrivals(targetBlockId, moved[moved.length - 1], reduceMotion);
    }
    return true;
}

/** The on-screen rectangle of each of the pane's pills, by block id; empty where the pane is not drawn. */
function pillRects(paneBlockId: string, blockIds: readonly string[]): Map<string, DOMRect> {
    const rects = new Map<string, DOMRect>();
    const pane = paneElement(paneBlockId);
    if (!pane) return rects;
    for (const el of pane.querySelectorAll<HTMLElement>("[data-tab-id]")) {
        const id = el.dataset.tabId;
        if (id && blockIds.includes(id)) rects.set(id, el.getBoundingClientRect());
    }
    return rects;
}

function paneElement(blockId: string): HTMLElement | null {
    if (typeof document === "undefined") return null;
    return document.querySelector<HTMLElement>(`[data-role="pane"][data-blockid="${CSS.escape(blockId)}"]`);
}

/**
 * FLIP: once the target's strip has drawn the arrived pills, move each from where it was in the
 * merged pane's header to its place, so the tabs travel into the neighbour instead of appearing in it.
 * The landing bounce (pane-tab-landing) plays on the pill itself, on top of the travel.
 */
function travelPills(targetBlockId: string, movedIds: readonly string[], from: Map<string, DOMRect>): void {
    if (!from.size) return;
    requestAnimationFrame(() => {
        const pane = paneElement(targetBlockId);
        if (!pane) return;
        for (const el of pane.querySelectorAll<HTMLElement>("[data-tab-id]")) {
            const id = el.dataset.tabId;
            const before = id && movedIds.includes(id) ? from.get(id) : undefined;
            if (!before) continue;
            const after = el.getBoundingClientRect();
            const dx = before.left - after.left;
            const dy = before.top - after.top;
            if (!dx && !dy) continue;
            el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "translate(0, 0)" }], {
                duration: DURATION_MODERATE_MS,
                easing: EASE_EMPHASIZED,
                composite: "add", // on top of the landing bounce's own transform
            });
        }
    });
}

/**
 * Scroll the target's strip to the end, where the arrived pills are, when it overflows: the merge
 * leaves the target's own tab visible, so the strip's selected-tab reveal never shows them. Runs
 * after the landing bounce, when the strip's own reveal (which waits that out) has had its turn,
 * and whether or not motion is reduced.
 */
function revealArrivals(targetBlockId: string, lastArrivedId: string, reduceMotion: boolean): void {
    if (typeof document === "undefined") return;
    setTimeout(() => {
        const pane = paneElement(targetBlockId);
        if (!pane) return;
        for (const pill of pane.querySelectorAll<HTMLElement>("[data-tab-id]")) {
            if (pill.dataset.tabId !== lastArrivedId) continue;
            const strip = pill.closest<HTMLElement>(".pane-tab-strip");
            if (strip) {
                revealInStrip(strip, pill, true, {
                    edgeClearance: STRIP_EDGE_FADE_PX,
                    behavior: reduceMotion ? "instant" : "smooth",
                });
            }
            return;
        }
    }, LANDING_BOUNCE_MS + 100);
}
