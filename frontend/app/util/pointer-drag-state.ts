// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tracks whether the mouse's primary button is currently held down —
 * i.e. a native text-selection drag (or any other primary-button drag)
 * may be in progress right now, anywhere in the app.
 *
 * Module-level singleton, not per-component state: shared by every
 * hover-triggered Portal-mounting UI in the app (`useNodePeek`'s
 * `PeekOverlay`, `element/tooltip.tsx`'s `Tooltip`, and any future one) to
 * gate mount/unmount while a drag is in progress. A row/anchor's
 * mouseenter/mouseleave still fires normally while the user is dragging
 * out a text selection (native selection-drag does not suppress hover
 * events), and letting either one through mid-drag mounts/unmounts a
 * Portal-rendered overlay under the cursor, which intermittently breaks
 * the browser's native selection-extend hit-testing — see
 * docs/plans/PLAN_AGENT_PANE_TEXT_SELECTION_DRAG_FLICKER_2026_09_20.md.
 *
 * Listens on `window` with `capture: true` so it observes the button
 * state regardless of which element the event actually lands on or
 * whether that element stops propagation.
 */

let primaryButtonDown = false;
/** Whether it was a mouse that pressed: only a mouse reports its buttons on every event that follows. */
let heldByMouse = false;
const releaseListeners = new Set<() => void>();

/** Where the mouse was last pressed, until it moves away from there. */
let pressPoint: { x: number; y: number } | null = null;
/** A hand does not hold the mouse dead still through a click: this much drift is still resting. */
const REST_SLOP_PX = 4;

const isAtPressPoint = (e: { clientX: number; clientY: number }): boolean =>
    pressPoint !== null && Math.abs(e.clientX - pressPoint.x) <= REST_SLOP_PX && Math.abs(e.clientY - pressPoint.y) <= REST_SLOP_PX;

function handlePointerDown(e: PointerEvent): void {
    // A tap's hover arrives after its press, at the same point, and is how touch shows a hover panel at all.
    pressPoint = e.pointerType === "touch" ? null : { x: e.clientX, y: e.clientY };
    if (e.button === 0) {
        primaryButtonDown = true;
        heldByMouse = e.pointerType === "mouse";
    }
}

/**
 * A release the page never heard. A press that opens something the system draws in a window of its own
 * (the list of a native select, a system menu) hands the rest of the gesture to that window: the page
 * gets no pointerup and no blur, and the button would read as held until the next full click. The next
 * mouse event says what the button really is, so it is believed over the press that was remembered.
 */
function noticeMissedRelease(e: PointerEvent): void {
    if (primaryButtonDown && heldByMouse && e.pointerType === "mouse" && (e.buttons & 1) === 0) released();
}

function handlePointerMove(e: PointerEvent): void {
    if (pressPoint !== null && !isAtPressPoint(e)) pressPoint = null;
    noticeMissedRelease(e);
}

/**
 * The wheel moves the page under a pointer that stays where it is. What comes under the pointer then was
 * brought there by the person, so it is a hover like any other: the press is forgotten.
 */
function handleWheel(): void {
    pressPoint = null;
}

function handlePointerRelease(e: PointerEvent): void {
    if (e.button !== 0) return;
    released();
}

function released(): void {
    primaryButtonDown = false;
    // Iterating the live Set on purpose, NOT a copy. Set iteration is
    // defined under concurrent deletion: an entry removed before it is
    // reached is simply not visited. That is the behaviour we want — if one
    // listener's teardown unsubscribes another (a component unmounting a
    // child), the unsubscribed one must not then be called. A defensive
    // `[...releaseListeners]` copy would call it anyway, which is worse, not
    // safer.
    for (const cb of releaseListeners) cb();
}

function handleWindowBlur(): void {
    // The mouse can be released outside the window (or the window can
    // lose focus mid-drag, e.g. alt-tab) without a pointerup ever
    // reaching us. Resetting on blur avoids getting stuck "down" forever.
    primaryButtonDown = false;
    pressPoint = null;
}

if (typeof window !== "undefined") {
    window.addEventListener("pointerdown", handlePointerDown, { capture: true });
    window.addEventListener("pointermove", handlePointerMove, { capture: true, passive: true });
    // An element's enter comes before the move that caused it, and hover panels decide on the enter.
    window.addEventListener("pointerover", noticeMissedRelease, { capture: true, passive: true });
    window.addEventListener("wheel", handleWheel, { capture: true, passive: true });
    window.addEventListener("pointerup", handlePointerRelease, { capture: true });
    window.addEventListener("pointercancel", handlePointerRelease, { capture: true });
    window.addEventListener("blur", handleWindowBlur);
}

export function isPrimaryButtonDown(): boolean {
    return primaryButtonDown;
}

/**
 * Whether the hover event `e` happened where the mouse was last pressed, with no move away since: the
 * pointer is resting on what it clicked. Such a hover is not the person pointing at something. The page
 * changed under the pointer (the click redrew the control, or put another in its place) and the browser
 * reports the new element as entered. Hover panels stay shut for it, as they do for the element that
 * was clicked, until the pointer leaves and comes back. Turning the wheel ends it: see `handleWheel`.
 *
 * It takes the event's own position because the browser sends an element's enter before the move that
 * caused it: a fast move from the press to another control must still count as a move.
 */
export function isRestingAtPress(e: { clientX: number; clientY: number }): boolean {
    return isAtPressPoint(e);
}

/** For tests: forget the last press, as a move away does. */
export function forgetPressPoint(): void {
    pressPoint = null;
}

/**
 * Run `cb` when the primary button is released. Returns an unsubscribe.
 *
 * Exists because gating hover on `isPrimaryButtonDown()` is only half a
 * rule. The gate drops the mouseenter that arrived mid-drag — and if the
 * drag ENDS with the cursor still sitting on the same anchor, no further
 * mouseenter/mouseleave ever fires for it, so the tooltip/peek stays shut
 * until the user leaves and re-enters. Consumers use this to re-check
 * `el.matches(":hover")` at the moment the drag ends and open if the
 * cursor is in fact still there (reagentx P2 on PR #3470, both call
 * sites).
 *
 * Deliberately NOT fired from `handleWindowBlur`: the button state is
 * reset there because we may never see the pointerup, but the window has
 * just lost focus, and `:hover` can still match — resyncing then would
 * pop an overlay onto a background window.
 */
export function onPrimaryButtonRelease(cb: () => void): () => void {
    releaseListeners.add(cb);
    return () => releaseListeners.delete(cb);
}
