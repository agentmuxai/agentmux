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
        // The pills travel first (as copies, see travelPills), then land with the bounce.
        const travelMs = from?.size ? DURATION_MODERATE_MS : 0;
        if (landedIn && !reduceMotion) {
            if (travelMs) travelPills(targetBlockId, moved, from!);
            if (travelMs) setTimeout(() => markLanded(moved, landedIn.id), travelMs);
            else markLanded(moved, landedIn.id);
        }
        revealArrivals(targetBlockId, moved[moved.length - 1], reduceMotion, travelMs);
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
 * Once the target's strip has drawn the arrived pills, a copy of each travels from where it was in
 * the merged pane's header to its place in the strip, so the tabs move into the neighbour instead
 * of appearing in it. Copies in a fixed layer, not the pills themselves: the strip clips its
 * contents, so a pill moved back to the other pane's header would be invisible until it entered
 * the strip. The real pills stay hidden meanwhile and land with the bounce (pane-tab-landing).
 */
function travelPills(targetBlockId: string, movedIds: readonly string[], from: Map<string, DOMRect>): void {
    requestAnimationFrame(() => {
        const pane = paneElement(targetBlockId);
        if (!pane) return;
        for (const el of pane.querySelectorAll<HTMLElement>("[data-tab-id]")) {
            const id = el.dataset.tabId;
            const before = id && movedIds.includes(id) ? from.get(id) : undefined;
            if (!before) continue;
            const after = el.getBoundingClientRect();
            const copy = el.cloneNode(true) as HTMLElement;
            copy.classList.remove("pane-tab--landing", "pane-tab--active");
            Object.assign(copy.style, {
                position: "fixed",
                left: `${before.left}px`,
                top: `${before.top}px`,
                width: `${after.width}px`,
                height: `${after.height}px`,
                margin: "0",
                pointerEvents: "none",
                zIndex: "10000",
            });
            document.body.append(copy);
            el.style.visibility = "hidden";
            const done = () => {
                copy.remove();
                el.style.visibility = "";
            };
            const travel = copy.animate(
                [{ transform: "translate(0, 0)" }, { transform: `translate(${after.left - before.left}px, ${after.top - before.top}px)` }],
                { duration: DURATION_MODERATE_MS, easing: EASE_EMPHASIZED }
            );
            travel.onfinish = done;
            travel.oncancel = done;
        }
    });
}

/**
 * Scroll the target's strip to the end, where the arrived pills are, when it overflows: the merge
 * leaves the target's own tab visible, so the strip's selected-tab reveal never shows them. Runs
 * after the travel and the landing bounce, when the strip's own reveal (which waits that out) has had its turn,
 * and whether or not motion is reduced.
 */
function revealArrivals(targetBlockId: string, lastArrivedId: string, reduceMotion: boolean, travelMs: number): void {
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
    }, travelMs + LANDING_BOUNCE_MS + 100);
}
