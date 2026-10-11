// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where a document tab dropped over a pane goes: near an edge, into a new pane
 * on that side (doc-tab-new-pane.ts); anywhere else, to the pane itself (a
 * document pane of the tab's type takes it, doc-tab-hosts.ts). Both ask this,
 * on the same tile, so they agree where one ends and the other begins.
 */

import { DropDirection } from "@/layout/lib/types";
import { determineDropDirection } from "@/layout/lib/utils";

export type SplitSide = "left" | "right" | "up" | "down";

const SIDE_OF: Partial<Record<DropDirection, SplitSide>> = {
    [DropDirection.OuterTop]: "up",
    [DropDirection.OuterRight]: "right",
    [DropDirection.OuterBottom]: "down",
    [DropDirection.OuterLeft]: "left",
};

/** The pane tile `el` is in (the layout's `.tile-node`), or `el` itself. */
export function tileOf(el: Element): Element {
    return el.closest(".tile-node") ?? el;
}

/**
 * The side a document tab dropped at (x, y) over `tile` splits off: within a
 * fifth of the tile's edge (the band in which a dragged pane goes beside one,
 * `determineDropDirection`'s Outer directions). Null anywhere else.
 */
export function docTabSplitSide(tile: Element, x: number, y: number): SplitSide | null {
    const r = tile.getBoundingClientRect();
    const dir = determineDropDirection({ width: r.width, height: r.height, left: r.left, top: r.top }, { x, y });
    return dir == null ? null : (SIDE_OF[dir] ?? null);
}
