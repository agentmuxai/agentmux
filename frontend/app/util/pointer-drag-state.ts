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
const releaseListeners = new Set<() => void>();

function handlePointerDown(e: PointerEvent): void {
    if (e.button === 0) primaryButtonDown = true;
}

function handlePointerRelease(e: PointerEvent): void {
    if (e.button !== 0) return;
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
}

if (typeof window !== "undefined") {
    window.addEventListener("pointerdown", handlePointerDown, { capture: true });
    window.addEventListener("pointerup", handlePointerRelease, { capture: true });
    window.addEventListener("pointercancel", handlePointerRelease, { capture: true });
    window.addEventListener("blur", handleWindowBlur);
}

export function isPrimaryButtonDown(): boolean {
    return primaryButtonDown;
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
