// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The doing half of the merge on minimize (the choice of target is layoutMinimizeMerge.ts). Kept
 * apart because it needs the stack mutators, whose imports (RPC, services) the layout's pure
 * modules must not pull in; layout/index.ts registers it with `setMinimizeMerger`.
 */

import { LANDING_BOUNCE_MS, markLanded } from "@/app/element/pane-tab-landing";
import { revealInStrip, STRIP_EDGE_FADE_PX } from "@/app/element/strip-reveal";
import { scaledMs, shouldReduceMotion } from "@/app/motion/motion";
import { mergeInto } from "@/app/motion/primitives";
import { DURATION_BASE_MS, DURATION_MODERATE_MS } from "@/app/motion/tokens";
import { findExpandedMergeTargets, findMinimizeMergeTarget } from "./layoutMinimizeMerge";
import type { LayoutModel } from "./layoutModel";
import { findNode, findNodeByBlockId } from "./layoutNode";
import { effectiveStack, moveBlocksInStack } from "./layoutStack";

/**
 * Merge the pane `nodeId` into its neighbours instead of minimizing it: into a minimized neighbour when
 * there is one (tabs that were hidden together stay together), else dealt out round robin to the
 * expanded panes (`findExpandedMergeTargets`: the nearest first, one tab each in strip order, round
 * again). The tabs are appended to the end of each target's strip, inactive (the target's own tab keeps
 * showing), and the emptied pane is removed as a move (its space goes to the other panes, as for a
 * minimize). Returns false, touching nothing, when there is no target or the first move could not be
 * applied, so the caller minimizes as before.
 */
export function mergeIntoNeighbour(model: LayoutModel, nodeId: string): boolean {
    const root = model.treeState.rootNode;
    const chip = findMinimizeMergeTarget(root, nodeId);
    const targets = chip ? [chip] : findExpandedMergeTargets(root, nodeId);
    const node = root && findNode(root, nodeId);
    if (!node?.data || targets.length === 0 || targets.some((t) => !t.data)) return false;
    const deals = dealOut(effectiveStack(node.data), targets.map((t) => t.data!.blockId));
    const reduceMotion = shouldReduceMotion("transition");
    const from = reduceMotion ? undefined : pillRects(node.data.blockId, [...deals.values()].flat());
    // The nearest pane's deal goes last, so the focus the last move leaves is where the first tab went.
    let any = false;
    for (const [targetBlockId, ids] of [...deals].reverse()) {
        const moved = moveBlocksInStack(model, ids, targetBlockId);
        if (!moved) continue;
        any = true;
        if (moved.length) animateArrival(model, targetBlockId, moved, from, reduceMotion);
    }
    return any;
}

/** `blockIds` dealt out round robin to `targetBlockIds` (the first tab to the first target); a target with no tab is left out. */
function dealOut(blockIds: readonly string[], targetBlockIds: readonly string[]): Map<string, string[]> {
    const deals = new Map<string, string[]>();
    blockIds.forEach((id, i) => {
        const target = targetBlockIds[i % targetBlockIds.length];
        deals.set(target, [...(deals.get(target) ?? []), id]);
    });
    return deals;
}

/** The pills that arrived in `targetBlockId` travel first (as copies, see travelPills), then land with the bounce. */
function animateArrival(
    model: LayoutModel,
    targetBlockId: string,
    moved: string[],
    from: Map<string, DOMRect> | undefined,
    reduceMotion: boolean
): void {
    const landedIn = model.treeState.rootNode && findNodeByBlockId(model.treeState.rootNode, targetBlockId);
    const travelMs = from?.size ? DURATION_MODERATE_MS : 0;
    if (landedIn && !reduceMotion) {
        if (travelMs) travelPills(targetBlockId, moved, from!, () => markLanded(moved, landedIn.id));
        else markLanded(moved, landedIn.id);
    }
    revealArrivals(targetBlockId, moved[moved.length - 1], reduceMotion, travelMs);
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

/**
 * The right edge, in viewport px, of where `el` can be seen: the nearest right edge among the ancestors that
 * clip it (the tab strip's scroller, the header, the pane). Undefined when nothing clips it.
 */
export function visibleRight(el: Element): number | undefined {
    let right: number | undefined;
    for (let up = el.parentElement; up; up = up.parentElement) {
        if (getComputedStyle(up).overflowX === "visible") continue;
        const r = up.getBoundingClientRect().right;
        right = right === undefined ? r : Math.min(right, r);
    }
    return right;
}

function paneElement(blockId: string): HTMLElement | null {
    if (typeof document === "undefined") return null;
    return document.querySelector<HTMLElement>(`[data-role="pane"][data-blockid="${CSS.escape(blockId)}"]`);
}

/**
 * Once the target's strip has drawn the arrived pills, a copy of each travels from where it was in
 * the merged pane's header to its place in the strip (motion primitive `mergeInto`), so the tabs
 * move into the neighbour instead of appearing in it. The copies live in a fixed layer, not in the
 * pills themselves: the strip clips its contents, so a pill moved back to the other pane's header
 * would be invisible until it entered the strip. The layer is cut off at the strip's right edge, so the pills
 * of a strip that is wider than its pane (those scrolled out of view) do not fly past the pane. The real
 * pills stay hidden meanwhile; `land` marks them for the bounce (pane-tab-landing) once the first arrives, and again if none could be found.
 */
function travelPills(
    targetBlockId: string,
    movedIds: readonly string[],
    from: Map<string, DOMRect>,
    land: () => void
): void {
    const ids = movedIds.filter((id) => from.has(id));
    let landed = false;
    const markOnce = () => {
        if (landed) return;
        landed = true;
        land();
    };
    const handle = mergeInto(
        ids.map((id) => from.get(id)!),
        () => {
            const pills = new Map<string, HTMLElement>();
            for (const el of paneElement(targetBlockId)?.querySelectorAll<HTMLElement>("[data-tab-id]") ?? []) {
                if (el.dataset.tabId) pills.set(el.dataset.tabId, el);
            }
            return ids.map((id) => pills.get(id));
        },
        {
            land: markOnce,
            // The copy is a picture of the pill, not an active or just-landed one.
            onCopy: (copy) => copy.classList.remove("pane-tab--landing", "pane-tab--active"),
            // A pill the strip has scrolled out of view flies toward its place and is cut off at the strip's edge, not
            // drawn across whatever lies beyond the pane.
            clipRight: visibleRight,
        }
    );
    // A pane that is gone, or pills that were not drawn: nothing flew, but the bounce still belongs to the move.
    void handle.finished.then(() => {
        if (!handle.cancelled) markOnce();
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
    setTimeout(
        () => {
            const pane = paneElement(targetBlockId);
            if (!pane) return;
            for (const pill of pane.querySelectorAll<HTMLElement>("[data-tab-id]")) {
                if (pill.dataset.tabId !== lastArrivedId) continue;
                const strip = pill.closest<HTMLElement>(".pane-tab-strip");
                if (strip) {
                    revealInStrip(strip, pill, true, {
                        edgeClearance: STRIP_EDGE_FADE_PX,
                        behavior: reduceMotion ? "instant" : "smooth",
                        // The same short tween as the strip's own reveal (scrolling at once when Animations are off).
                        durationMs: scaledMs(DURATION_BASE_MS),
                    });
                }
                return;
            }
        },
        // The pills' travel and bounce are scaled, so the wait for them is too.
        scaledMs(travelMs + LANDING_BOUNCE_MS + 100)
    );
}
