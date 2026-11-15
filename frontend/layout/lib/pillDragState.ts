// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The tab pill that is being placed as a pane of its own, if one is: a single pane tab dragged over a pane's
 * body in its own layout (pillPlacement.ts). While it is, the layout shows the drop as it shows a whole
 * pane's (the live reflow, liveReflow.ts), and the pointer is read against the layout's own rects, not against
 * the panes on screen, which the reflow has moved.
 *
 * Only the state, with no dependencies, so the modules that show a drop (placementPreview.ts, liveReflow.ts,
 * settlingPass.ts) can read it without the ones that follow the pointer.
 */

import { createSignal } from "solid-js";
import type { LayoutModel } from "./layoutModel";
import type { DropDirection, LayoutNode } from "./types";

export interface PillDrag {
    model: LayoutModel;
    /** The dragged tab's block. */
    blockId: string;
    /** The pane the tab is in. */
    sourceNodeId: string;
    /** The tab is its pane's only one: placing it moves the pane itself. */
    lone: boolean;
    /** The leaf that stands for the new pane in a preview: one object for the whole drag. */
    standIn: LayoutNode;
    /** What the drop is tracked and previewed as: the pane itself for a lone tab, the stand-in otherwise. */
    draggedId: string;
    /** The drop that is shown, which a release commits. */
    armed?: { targetId: string; direction: DropDirection };
    /** Released over the layout: the placing ends when the drop has been committed, not when the drag does. */
    dropped?: boolean;
}

const [pill, setPill] = createSignal<PillDrag | undefined>(undefined);

/** The tab being placed in `model`'s layout, or none. Reactive. */
export function pillDragIn(model: LayoutModel): PillDrag | undefined {
    const p = pill();
    return p?.model === model ? p : undefined;
}

/** Whether a tab is being placed in `model`'s layout. Reactive. */
export const pillPlacingIn = (model: LayoutModel): boolean => pillDragIn(model) !== undefined;

/** The tab being placed, in whichever layout. Not for rendering: read once. */
export const currentPillDrag = (): PillDrag | undefined => pill();

const [reflowOf, setReflowOf] = createSignal<LayoutModel | undefined>(undefined);
let lingering: ReturnType<typeof setTimeout> | undefined;

/**
 * Whether `model`'s layout reflows for a tab pill now: while one is placed in it, and for `lingerMs` after a
 * placing that ended with no drop, so the panes glide home as they do when a pane drag's drop is taken down
 * (a reflow that stopped at once would put them back in one step). Reactive.
 */
export const pillReflowIn = (model: LayoutModel): boolean => reflowOf() === model;

/**
 * Start a placing (`next`), or end the one that is on. `lingerMs` keeps the reflow on that long after the end;
 * without it the reflow ends with the placing, as it must when the drop has just been committed.
 */
export function setPillDrag(next: PillDrag | undefined, lingerMs = 0): void {
    clearTimeout(lingering);
    setPill(next);
    if (next) setReflowOf(next.model);
    else if (lingerMs > 0) lingering = setTimeout(() => setReflowOf(undefined), lingerMs);
    else setReflowOf(undefined);
}
