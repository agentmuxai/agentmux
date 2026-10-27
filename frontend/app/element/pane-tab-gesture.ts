// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a click on a pane tab pill means, from the gesture that made it.
 *
 * A pill is also a drag source, and Ctrl/Cmd+click picks it for a multi-tab drag, so a click is
 * only a plain click when nothing else was going on: no drag started or just ended, the pointer
 * stayed put between press and release, and no modifier was held. Only a plain click expands a
 * collapsed pane (the operator, 2026-10-27: a drag between minimized panes must not open the
 * pane; the pane expands only on a plain click).
 */

/** The pointer may move this far between press and release and still be a click. */
export const PILL_DRAG_THRESHOLD_PX = 5;

/** The button whose plain click expands a collapsed pane: the left one. */
export const PILL_EXPAND_BUTTON = 0;

/** A click this soon after a pill drag ended belongs to the drag, not to a new gesture. */
export const POST_DRAG_CLICK_GUARD_MS = 300;

export type PillClickIntent =
    /** Plain click: expand a collapsed pane, focus it, activate the tab. */
    | "expand"
    /** Shift or Alt held: activate the tab, but expand nothing. */
    | "activate-only"
    /** Ctrl/Cmd held: pick or unpick the pill for a multi-tab drag, and nothing else. */
    | "toggle-selection"
    /** Not a click of ours (another button, or the end of a drag): do nothing. */
    | "ignore";

export interface PillClickGesture {
    button: number;
    /** Ctrl (Cmd on macOS) held. */
    mod: boolean;
    shift: boolean;
    alt: boolean;
    /** Where the button went down on this pill, or null if no press was seen. */
    pressedAt: { x: number; y: number } | null;
    /** Where the click landed. */
    clickedAt: { x: number; y: number };
    /** A drag started on this pill since the press. */
    dragStartedSincePress: boolean;
    /** Milliseconds since the last pill drag ended anywhere, or null if none has. */
    msSinceLastPillDrag: number | null;
}

/** Whether the gesture was a drag, or the click a drag leaves behind. */
export function wasDrag(g: PillClickGesture): boolean {
    if (g.dragStartedSincePress) return true;
    if (g.msSinceLastPillDrag !== null && g.msSinceLastPillDrag < POST_DRAG_CLICK_GUARD_MS) return true;
    if (!g.pressedAt) return false;
    return Math.hypot(g.clickedAt.x - g.pressedAt.x, g.clickedAt.y - g.pressedAt.y) > PILL_DRAG_THRESHOLD_PX;
}

export function classifyPillClick(g: PillClickGesture): PillClickIntent {
    if (g.button !== PILL_EXPAND_BUTTON) return "ignore";
    if (wasDrag(g)) return "ignore";
    if (g.mod) return "toggle-selection";
    if (g.shift || g.alt) return "activate-only";
    return "expand";
}

let lastPillDragEndedAt: number | null = null;

/** A pill drag ended (dropped, cancelled, or released on nothing). */
export function markPillDragEnded(now: number = Date.now()): void {
    lastPillDragEndedAt = now;
}

/** Milliseconds since the last pill drag ended, or null if none has. */
export function msSinceLastPillDrag(now: number = Date.now()): number | null {
    return lastPillDragEndedAt === null ? null : now - lastPillDragEndedAt;
}

/** For tests. */
export function resetPillDragClock(): void {
    lastPillDragEndedAt = null;
}
