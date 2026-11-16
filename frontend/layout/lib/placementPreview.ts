// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where a dragged pane or pane tab would be placed next to the pane under the pointer, and the
 * ghost that shows it. The one place a pointer position becomes a `DropDirection` and a pending
 * action: the tile overlay (`OverlayNode`, and its dead-spot fallback) and a tab pill dragged over a
 * pane's body (`paneBodyDrop`) both call it, so the ghost is the standard one.
 * docs/specs/SPEC_PANE_TAB_DRAG_BODY_PLACEMENT_AND_EDGE_GLOW_2026_10_29.md §3.2.
 *
 * The pointer is read against the hot spots of the hovered pane's guide (dropGuides.ts), one per layout a
 * release there can leave (dropOptions.ts, which also makes the action each one commits). Off the spots a
 * pane has no drop.
 *
 * It is also the one place a shown drop is taken down again, and the one place that says when: `clearPlacement`
 * at once, `leavePlacement` after the grace a pointer needs to cross from one pane's zones to the next.
 * Everything a drag keeps between samples (the steadied candidate, its dwell timer, a leave about to clear)
 * is here, so no caller can take a drop down and leave a timer behind that brings it back. A caller never
 * dispatches `ComputeMove` or `ClearPendingAction` for a drop itself. What a drag waits for, in one list:
 * `DEFAULT_TRACKER` (dropIntentTracker.ts: how long a new candidate must hold), `LEAVE_GRACE_MS`, and
 * `ZONE_EXIT_MS` (how long a zone the pointer left takes to fade).
 */

import { ELEMENT_DRAG_KINDS, isAnyUnderway, onSessionEnded } from "@/app/drag/drag-session";
import { fadeFollowedPane } from "@/app/drag/pane-live-follow";
import { chipOverChipDirection } from "./chipPlaceholder";
import { DEFAULT_TRACKER, DropIntentTracker } from "./dropIntentTracker";
import { dropHeldIn, forgetHeldDrop } from "./heldDrop";
import { dragState } from "./tilelayout-drag-state";
import type { LayoutModel } from "./layoutModel";
import { pillDragIn } from "./pillDragState";
import { leaveOwnPane, pointerOutsideWindow } from "./tearOffPreview";
import {
    LayoutTreeActionType,
    type DropDirection,
    type LayoutTreeComputeMoveNodeAction,
    type LayoutTreeSetPendingAction,
    type LayoutTreeStayAction,
} from "./types";
import { GUIDE_GRACE_MS, activeFor, guideSpotsFor, hideAllGuides, hideGuide, markGuideActive, onGuidesShown, showGuide, spotAt } from "./dropGuides";
import { computeFor, dropOptionsOf, dropsBySplit, layoutShape, resetDropOptions, splitFor, type DropSubject } from "./dropOptions";

export { layoutShape, sideOfDirection, type PlacementSide } from "./dropOptions";

export interface PlacementInput extends DropSubject {
    /** The hovered pane's rectangle in the layout container's coordinates. */
    rect: Dimensions;
    /** The pointer, in the layout container's coordinates. */
    offset: { x: number; y: number };
}

/**
 * The direction of the drop under the pointer, or undefined where there is none. A drop is under the pointer
 * only on its spot of the hovered pane's guide (dropGuides.ts), and this is where that guide is shown: every
 * reader of a drag comes through here, so the spots on screen are the ones the pointer is read against. A chip
 * over a chip keeps its own rule (the chip's halves), with no guide.
 */
export function placementDirection(model: LayoutModel, input: PlacementInput): DropDirection | undefined {
    const inTree = !input.foreignNode && !input.pill;
    const chip = inTree
        ? chipOverChipDirection(model.treeState.rootNode, input.targetId, input.draggedId, input.rect, input.offset)
        : undefined;
    if (chip !== undefined) {
        hideGuide(model);
        return chip;
    }
    const spots = guideSpotsFor(model, input.rect, input.targetId, dropOptionsOf(model, input));
    const direction = spotAt(spots, input.offset)?.direction;
    showGuide(model, { targetId: input.targetId, spots, active: activeFor(spots, direction) });
    return direction;
}

/**
 * Show the ghost for `direction` over `input.targetId`: dispatches `ComputeMove`, and the layout's
 * placeholder draws the pending action. Returns the action it dispatched.
 */
export function showPlacement(
    model: LayoutModel,
    input: PlacementInput,
    direction: DropDirection | undefined
): LayoutTreeComputeMoveNodeAction {
    const compute = computeFor(input, direction);
    // A sample the throttle held back past the pointer's exit: the pointer is outside now, and what the
    // window shows is the tear-off (tearOffPreview.ts), which a placement would replace. Coming back is
    // the window's `dragenter`, not a stale sample.
    if (pointerOutsideWindow(model)) return compute;
    // The pointer is over another pane: no tear-off is shown.
    leaveOwnPane(model);
    // A drop is shown again: the zones the pointer left a moment ago clear nothing (`leavePlacement`).
    cancelLeave(model);
    // And it is this drag's: a drop still held from the last one (heldDrop.ts) is no longer what is shown.
    forgetHeldDrop(model);
    if (dropsBySplit(input)) {
        const split = splitFor(input, direction);
        model.treeReducer(
            split
                ? ({ type: LayoutTreeActionType.SetPendingAction, action: split } as LayoutTreeSetPendingAction)
                : { type: LayoutTreeActionType.ClearPendingAction }
        );
    } else if (direction === undefined) {
        // Off every spot: no drop is shown, and the tree is not asked for a move in no direction.
        model.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
    } else {
        model.treeReducer(compute);
    }
    return compute;
}

/** The drop now shown for the tab pill being placed, which its release commits. */
function notePillDrop(model: LayoutModel, input: PlacementInput, armed: { targetId: string; direction: DropDirection | undefined }): void {
    const pill = input.pill ? pillDragIn(model) : undefined;
    if (pill) pill.armed = armed.direction === undefined ? undefined : { targetId: armed.targetId, direction: armed.direction };
}

/**
 * Show the dragged pane's own place as its drop: the layout as it is, with the pane where it was under the
 * landing zone. For a drag whose own place has become a drop like any other (`ownPlaceDrops`, futureLayout.ts);
 * before that the place tears the pane off and shows no drop (tearOffPreview.ts). It goes through the pending
 * action as every drop does (`Stay`), so it is replaced, cleared and committed as one, and committed it
 * changes nothing. No candidate is held from before: the next pane's zones arm at once.
 */
export function showStay(model: LayoutModel, nodeId: string): void {
    if (pointerOutsideWindow(model)) return;
    leaveOwnPane(model);
    resetDropIntent(model);
    cancelLeave(model);
    // Over the dragged pane's own place: no other pane's guide stays.
    hideGuide(model);
    const stay: LayoutTreeStayAction = { type: LayoutTreeActionType.Stay, nodeId };
    model.treeReducer({ type: LayoutTreeActionType.SetPendingAction, action: stay } as LayoutTreeSetPendingAction);
}

/** What a model's drag is steadied by: its tracker, the latest input per hovered pane, and the dwell timer. */
interface Steadying {
    tracker: DropIntentTracker;
    cross: boolean;
    inputs: Map<string, PlacementInput>;
    timer?: ReturnType<typeof setTimeout>;
    /** The layout the armed candidate belongs to (`layoutShape`): another one means the tree changed under the drag. */
    shape?: string;
}

const steadying = new WeakMap<LayoutModel, Steadying>();
/** The models with a drag being steadied, so the end of a drag can reset the destination tab's too. */
const steadied = new Set<LayoutModel>();

function steadyingOf(model: LayoutModel, cross: boolean): Steadying {
    const have = steadying.get(model);
    if (have && have.cross === cross) return have;
    if (have) clearTimeout(have.timer);
    // A pane from another tab arms at once, however fast the pointer: its drop record is taken with every sample.
    const config = cross ? { ...DEFAULT_TRACKER, dwellMs: 0, fastPxPerMs: Infinity } : DEFAULT_TRACKER;
    const made = { tracker: new DropIntentTracker(config), cross, inputs: new Map<string, PlacementInput>() };
    steadying.set(model, made);
    steadied.add(model);
    return made;
}

/** The drag is over: the next one starts from nothing, and no waiting candidate shows a ghost after the drop. */
export function resetDropIntent(model: LayoutModel): void {
    const have = steadying.get(model);
    if (!have) return;
    clearTimeout(have.timer);
    steadying.delete(model);
    steadied.delete(model);
}

/**
 * Clear the ghost. A candidate still waiting must not bring it back, so its timer goes too; `reset` also drops
 * the armed candidate (the pointer is over the dragged pane itself, where there is none).
 *
 * A drop that was released and that srv has still to commit stays shown (heldDrop.ts): everything the drag
 * kept goes, and the drop itself is taken down by the layout that brings the pane. So whichever handler of
 * the ended drag comes here, and in whichever order, none of them takes the landing back.
 */
export function clearPlacement(model: LayoutModel, reset = false): void {
    if (reset) resetDropIntent(model);
    else clearTimeout(steadying.get(model)?.timer);
    cancelLeave(model);
    // The next sample shows the guide again within a frame when the pointer is still over a pane.
    hideGuide(model, isAnyUnderway(ELEMENT_DRAG_KINDS) ? GUIDE_GRACE_MS : 0);
    if (dropHeldIn(model)) return;
    model.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
}

/** How long a landing zone the pointer has left takes to go, while the drag is still on, ms (tilelayout.scss `.exiting`). */
export const ZONE_EXIT_MS = 150;

/**
 * How long a drop stays shown after the pointer left the zones it was armed from, ms. Crossing from one pane's
 * zones to the next is a leave and then, a frame later, the next pane's first sample, which most often shows
 * the same drop again (the tracker holds it). Cleared at the leave, the drop was gone for that frame: the panes
 * started back to their own places, the pane in hand went back to its full size, the landing zone began its
 * exit with the layout dimmed around it, and all of it came forward again (the operator, 2026-11-12: "it snaps
 * in and snaps out really fast into a possible panel layout"; "a window-tab wide fade-in/out, very fast").
 */
export const LEAVE_GRACE_MS = 80;

/** The layouts whose drop a leave is about to clear, with the timer that does it. */
const leaves = new Map<LayoutModel, ReturnType<typeof setTimeout>>();

function cancelLeave(model: LayoutModel): void {
    const timer = leaves.get(model);
    if (timer === undefined) return;
    clearTimeout(timer);
    leaves.delete(model);
}

/**
 * The pointer left the zones a drop was shown from (a pane's overlay, or the layout itself). The drop is
 * cleared once `LEAVE_GRACE_MS` has passed with no sample showing one again: a pointer that only crossed to
 * the next pane's zones sends one within a frame, and one that has really left sends none. A candidate still
 * waiting must not bring the ghost back, so its timer goes at once. With no drag underway (the leave that a
 * cancelled drag ends with) there is nothing to wait for.
 */
export function leavePlacement(model: LayoutModel): void {
    if (!isAnyUnderway(ELEMENT_DRAG_KINDS)) {
        clearPlacement(model);
        return;
    }
    clearTimeout(steadying.get(model)?.timer);
    cancelLeave(model);
    leaves.set(
        model,
        setTimeout(() => clearPlacement(model), LEAVE_GRACE_MS)
    );
}

// The picture of the dragged pane that the cursor carries (pane-live-follow.ts) is over the page, guides and all:
// it fades while a layout shows a guide, so the spots are seen through it.
onGuidesShown(fadeFollowedPane);

// The drag is over. A drop has committed what was shown and cleared it; a cancelled drag left it to the grace,
// and nothing shows a drop of a drag that is over.
onSessionEnded(() => {
    for (const model of [...leaves.keys()]) clearPlacement(model);
    hideAllGuides();
    resetDropOptions();
});

/** The drag is over, in whichever tab it was steadied (a pane dragged to another tab has two). */
export function resetAllDropIntent(): void {
    for (const model of [...steadied]) resetDropIntent(model);
}

/**
 * `placementDirection` and `showPlacement` in one step, for a drag that previews as it moves. The ghost
 * follows the steadied candidate (dropIntentTracker.ts), which can be another pane's than the one the
 * pointer is over: `targetId` says which, and is what a caller records a drop against.
 */
export function previewPlacement(
    model: LayoutModel,
    input: PlacementInput
): { direction: DropDirection | undefined; compute: LayoutTreeComputeMoveNodeAction; targetId: string } {
    const raw = { targetId: input.targetId, direction: placementDirection(model, input) };
    const held = steadyingOf(model, !!input.foreignNode && !input.pill);
    clearTimeout(held.timer);
    const shape = layoutShape(model);
    if (held.shape !== undefined && held.shape !== shape) {
        // The tree changed under the drag: the armed candidate and the inputs describe panes that moved.
        held.tracker.reset();
        held.inputs.clear();
    }
    held.shape = shape;
    held.inputs.set(input.targetId, input);
    const now = performance.now();
    const { armed, flushAt } = held.tracker.step(input.draggedId, input.offset.x, input.offset.y, now, raw);
    if (flushAt !== undefined) {
        held.timer = setTimeout(() => showWaiting(model, held, input.draggedId), Math.max(0, flushAt - now));
    }
    const shown = held.inputs.get(armed.targetId) ?? input;
    notePillDrop(model, shown, armed);
    markGuideActive(model, armed.targetId, armed.direction);
    return { direction: armed.direction, compute: showPlacement(model, shown, armed.direction), targetId: armed.targetId };
}

/** The dwell ran out with no new sample: show the candidate that was waiting. */
function showWaiting(model: LayoutModel, held: Steadying, draggedId: string): void {
    if (dragState.nodeId !== draggedId && pillDragIn(model)?.draggedId !== draggedId) return;
    const armed = held.tracker.armWaiting();
    const shown = armed && held.inputs.get(armed.targetId);
    if (!armed || !shown) return;
    notePillDrop(model, shown, armed);
    markGuideActive(model, armed.targetId, armed.direction);
    showPlacement(model, shown, armed.direction);
}

/**
 * The pointer was released: the travel is over, as it is when the dwell runs out with no new sample, so the
 * candidate that was waiting for the pointer to stop is the drop. Nothing changes when none was waiting (the
 * shown drop is the one under the pointer, or within the margin that keeps it).
 */
export function settleDropIntent(model: LayoutModel, draggedId: string): void {
    const held = steadying.get(model);
    if (!held) return;
    clearTimeout(held.timer);
    showWaiting(model, held, draggedId);
}
