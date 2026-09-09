// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Redock drop guides — the explicit targets a floating pane must be aimed at
 * before it will dock, and the neutral area around them where it will not.
 *
 * WHY THIS EXISTS, rather than reusing `determineDropDirection`:
 * that function partitions the WHOLE leaf — every pixel resolves to some
 * direction, by design, because an in-window tile drag is already a committed
 * move and only has to decide where. A floating pane is different: "leave it
 * floating above the window" is a legitimate outcome, and with a total
 * partition there is nowhere to express it. That is why a floater could not be
 * parked over AgentMux no matter how the dwell was tuned — the dwell decides
 * *when* the drag becomes a dock, but something has to decide *whether*.
 * See `docs/specs/SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md` §3.3/§5.3.
 *
 * `determineDropDirection` is deliberately left alone: it is shared with the
 * in-window tile drag, where a total partition is the correct behaviour.
 *
 * Layout, per hovered leaf:
 *   - a plus-shaped compass of five cells at the centre (Center + the four
 *     half-splits), and
 *   - a thin band along each of the four edges (the Outer* narrow splits).
 * Everything else — including the diagonal corners of the compass — is
 * parking area. All nine `DropDirection` values stay reachable, so this
 * removes no capability.
 */

import { DropDirection } from "@/layout/lib/types";

export interface LeafBox {
    top: number;
    left: number;
    width: number;
    height: number;
}

export interface GuideRect extends LeafBox {
    dir: DropDirection;
}

/** Edge in CSS px. Deliberately a fixed size, not a fraction of the leaf: a
 *  guide is a UI target the user aims at, and targets should not grow with the
 *  pane. The old fractional bands were a third of a large pane each. */
const EDGE_BAND = 24;
/** One compass cell, CSS px. */
const CELL = 44;
/** Space between compass cells, CSS px. */
const GAP = 4;

/**
 * Guides for one leaf, in the same client-CSS-px space as the leaf rect.
 *
 * Compass cells come first: `hitTestGuides` returns the first match, so on a
 * leaf small enough that the clamped compass meets the clamped edge bands, the
 * centre still means Center rather than an edge split.
 */
export function guideRectsForLeaf(leaf: LeafBox): GuideRect[] {
    const shortest = Math.min(leaf.width, leaf.height);
    // Clamp both features so they stay inside a small pane and never meet in
    // the middle. A third of the shortest side leaves room for the neutral
    // ring at any size the layout can actually produce.
    const cell = Math.max(8, Math.min(CELL, (shortest - 2 * GAP) / 3 - 2));
    const band = Math.max(4, Math.min(EDGE_BAND, shortest / 6));

    const midX = leaf.left + leaf.width / 2;
    const midY = leaf.top + leaf.height / 2;
    const step = cell + GAP;
    const half = cell / 2;

    const cellAt = (dir: DropDirection, cx: number, cy: number): GuideRect => ({
        dir,
        left: cx - half,
        top: cy - half,
        width: cell,
        height: cell,
    });

    return [
        cellAt(DropDirection.Center, midX, midY),
        cellAt(DropDirection.Top, midX, midY - step),
        cellAt(DropDirection.Bottom, midX, midY + step),
        cellAt(DropDirection.Left, midX - step, midY),
        cellAt(DropDirection.Right, midX + step, midY),
        {
            dir: DropDirection.OuterTop,
            left: leaf.left,
            top: leaf.top,
            width: leaf.width,
            height: band,
        },
        {
            dir: DropDirection.OuterBottom,
            left: leaf.left,
            top: leaf.top + leaf.height - band,
            width: leaf.width,
            height: band,
        },
        {
            dir: DropDirection.OuterLeft,
            left: leaf.left,
            top: leaf.top,
            width: band,
            height: leaf.height,
        },
        {
            dir: DropDirection.OuterRight,
            left: leaf.left + leaf.width - band,
            top: leaf.top,
            width: band,
            height: leaf.height,
        },
    ];
}

/** The guide under this point, or null when the cursor is in parking area. */
export function hitTestGuides(
    guides: GuideRect[],
    x: number,
    y: number,
): DropDirection | null {
    for (const g of guides) {
        if (x >= g.left && x <= g.left + g.width && y >= g.top && y <= g.top + g.height) {
            return g.dir;
        }
    }
    return null;
}
