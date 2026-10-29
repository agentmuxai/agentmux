// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The slide a Swarm row makes when the order moves it
 * (docs/specs/SPEC_SWARM_ORDER_WORKING_FIRST_THEN_RECENT_2026_10_23.md §4). The rows
 * are measured before the move and after it, and each one that changed place slides
 * from where it was, through the motion primitive `move`.
 */

import { capture, move, type Snapshot } from "@/app/motion/primitives";

const KEY_ATTR = "data-swarm-key";

const keyOf = (row: Element) => row.getAttribute(KEY_ATTR);

function rows(list: HTMLElement | undefined): HTMLElement[] {
    return list ? [...list.querySelectorAll<HTMLElement>(`:scope > [${KEY_ATTR}]`)] : [];
}

/** Where each row is, by key, as the rows are now. */
export function captureRows(list: HTMLElement | undefined): Snapshot {
    return capture(rows(list), keyOf);
}

/**
 * Slide every row that is now somewhere else from where `before` had it. `zoom` is the
 * pane's zoom: the rows sit inside a zoomed box, and a transform is in that box's units.
 * Does nothing when the motion policy asks for no motion for a transition, or the engine
 * cannot animate.
 */
export function playRowMoves(list: HTMLElement | undefined, before: Snapshot, zoom = 1): void {
    move(before, rows(list), keyOf, { axis: "y", zoom });
}
