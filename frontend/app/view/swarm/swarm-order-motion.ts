// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The slide a Swarm row makes when the order moves it
 * (docs/specs/SPEC_SWARM_ORDER_WORKING_FIRST_THEN_RECENT_2026_10_23.md §4). The rows
 * are measured before the move and after it, and each one that changed place slides
 * from where it was.
 */

import { shouldReduceMotion } from "@/app/motion/motion";

/** How long a row takes to slide into its new place, in ms. */
export const SWARM_SLIDE_MS = 180;

const KEY_ATTR = "data-swarm-key";

function rows(list: HTMLElement | undefined): HTMLElement[] {
    return list ? [...list.querySelectorAll<HTMLElement>(`:scope > [${KEY_ATTR}]`)] : [];
}

/** The top of each row, by key, as the rows are now. */
export function captureRowTops(list: HTMLElement | undefined): Map<string, number> {
    const tops = new Map<string, number>();
    for (const row of rows(list)) tops.set(row.getAttribute(KEY_ATTR)!, row.getBoundingClientRect().top);
    return tops;
}

/**
 * Slide every row that is now somewhere else from where `before` had it. `zoom` is the
 * pane's zoom: the rows sit inside a zoomed box, and a transform is in that box's units.
 * Does nothing when the motion policy asks for no motion for a transition, or the engine
 * cannot animate.
 */
export function playRowMoves(list: HTMLElement | undefined, before: ReadonlyMap<string, number>, zoom = 1): void {
    if (shouldReduceMotion("transition")) return;
    for (const row of rows(list)) {
        if (typeof row.animate !== "function") return;
        const was = before.get(row.getAttribute(KEY_ATTR)!);
        if (was === undefined) continue;
        const moved = (was - row.getBoundingClientRect().top) / (zoom || 1);
        if (Math.abs(moved) < 1) continue;
        row.animate([{ transform: `translateY(${moved}px)` }, { transform: "translateY(0)" }], {
            duration: SWARM_SLIDE_MS,
            easing: "ease-out",
        });
    }
}
