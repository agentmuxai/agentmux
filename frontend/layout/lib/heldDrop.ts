// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A drop that srv commits: a pane, or a pane tab placed as a pane, that comes from another window tab
 * (crossTabDrag.ts). Its release changes nothing in this layout's tree. The pane arrives a moment later, with
 * the layout srv sends.
 *
 * A drop inside one tab commits what the panes already show, so nothing moves at the release. This one was
 * taken down at the release like a drop that was not made: the panes went back to their places, and when the
 * layout arrived they were sent out again to the same places, with the new pane fading in (the operator,
 * 2026-11-15: "a pane will land, and the process ends successfully except for a 2nd repeat uncessary
 * animation").
 *
 * So the drop that was shown stays shown until the layout holds the pane, and the layout's arrival is then
 * the commit of what is on screen, as it is inside one tab:
 * - `clearPlacement` (placementPreview.ts), the one place a shown drop is taken down, leaves a held one
 *   standing, so no handler of the drag that has just ended can take it down;
 * - the reflow stays on and no settling pass opens (liveReflow.ts `reflowDragIn`);
 * - the layout pass that brings the pane ends the hold in its own batch (`landHeldDrop`).
 * A pane that does not come ends it too: srv refused the move, or `HELD_DROP_MAX_MS` passed.
 *
 * Only state and the pending action, so the modules that show a drop can all read it.
 */

import { batch, createSignal, untrack } from "solid-js";
import type { LayoutModel } from "./layoutModel";
import { LayoutTreeActionType, type LayoutNode } from "./types";

interface HeldDrop {
    model: LayoutModel;
    /** The block the pane arrives with: the first of the tabs that were moved. */
    blockId: string;
}

/** How long a drop is held for a pane that has not come. */
export const HELD_DROP_MAX_MS = 2000;

const [held, setHeld] = createSignal<HeldDrop | undefined>(undefined);
let giveUp: ReturnType<typeof setTimeout> | undefined;

/** Whether `model`'s layout shows a drop that srv has still to commit. Reactive. */
export const dropHeldIn = (model: LayoutModel): boolean => held()?.model === model;

/**
 * The drop shown in `model`'s layout was released, and srv will bring the pane of `blockId`: keep it shown
 * until then. False, holding nothing, when no drop is shown there (a release on the window tab's button).
 */
export function holdDrop(model: LayoutModel, blockId: string): boolean {
    if (!untrack(() => model.futureLayout?.())) return false;
    clearTimeout(giveUp);
    setHeld({ model, blockId });
    giveUp = setTimeout(() => releaseHeldDrop(model), HELD_DROP_MAX_MS);
    return true;
}

/** The held drop is no longer the one shown (another drag shows its own): hold nothing, and leave what is shown. */
export function forgetHeldDrop(model: LayoutModel): void {
    if (untrack(held)?.model !== model) return;
    clearTimeout(giveUp);
    setHeld(undefined);
}

/** The pane is not coming: take the held drop down, and the panes go back to their places. */
export function releaseHeldDrop(model: LayoutModel): void {
    if (untrack(held)?.model !== model) return;
    clearTimeout(giveUp);
    batch(() => {
        setHeld(undefined);
        model.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
    });
}

const holdsBlock = (leaf: LayoutNode, blockId: string): boolean =>
    leaf.data?.blockId === blockId || !!leaf.data?.blockStack?.includes(blockId);

/**
 * The layout pass of `model` is about to publish `nextLeafs` (`updateTree`, inside its batch, before the
 * write): when they hold the pane the held drop waits for, the drop is committed and the hold is over. In the
 * same batch, so the panes go from the rects the drop showed to their own, which are the same, in one frame.
 */
export function landHeldDrop(model: LayoutModel, nextLeafs: LayoutNode[]): void {
    const drop = untrack(held);
    if (drop?.model !== model || !nextLeafs.some((leaf) => holdsBlock(leaf, drop.blockId))) return;
    clearTimeout(giveUp);
    setHeld(undefined);
    // Not through the reducer: this is inside its layout pass.
    model.pendingTreeAction.throttledValueAtom._set(undefined);
}

/** Test hook. */
export function resetHeldDropForTests(): void {
    clearTimeout(giveUp);
    setHeld(undefined);
}
