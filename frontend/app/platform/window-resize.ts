// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether the app window is being resized right now.
 *
 * While it is, work that doesn't change what's on screen can wait: window
 * tabs kept laid out but not shown skip layout (workspace.tsx), because laying
 * every tab out on every frame of a drag starved the renderer — 30 frames
 * instead of ~96 with 16 terminals across five tabs, and the newly exposed
 * area showed the page background until the frame arrived
 * (docs/analysis/ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md).
 *
 * The browser's `resize` event drives it: it fires once per frame for a drag,
 * a maximize, a snap or a programmatic resize alike, and before that frame's
 * style and layout, so the first frame of a resize already benefits.
 */

import { createSignal, untrack } from "solid-js";

/** How long after the last `resize` event the window counts as settled. */
export const WINDOW_RESIZE_SETTLE_MS = 120;

const [resizing, setResizing] = createSignal(false);
let settleTimer: ReturnType<typeof setTimeout> | null = null;

/** True from the first `resize` event until `WINDOW_RESIZE_SETTLE_MS` after the last. */
export function windowResizing(): boolean {
    return resizing();
}

/** Record one resize step. Exported for tests and for resizes not seen as a `resize` event. */
export function noteWindowResize(): void {
    cancelDrain?.();
    cancelDrain = null;
    if (!resizing()) setResizing(true);
    if (settleTimer != null) clearTimeout(settleTimer);
    settleTimer = setTimeout(() => {
        settleTimer = null;
        setResizing(false);
        drainBehind();
    }, WINDOW_RESIZE_SETTLE_MS);
}

// Panes that skipped layout during a resize (a minimized pane's content, tilelayout.scss) and haven't caught up.
// They catch up one per idle period once the window settles, not all in one long frame.
const [behind, setBehind] = createSignal<ReadonlySet<string>>(new Set());
let cancelDrain: (() => void) | null = null;

/** A pane skipped layout during the resize: it counts as behind until it has caught up. */
export function markLayoutBehind(id: string): void {
    if (untrack(behind).has(id)) return;
    setBehind(new Set(untrack(behind)).add(id));
}

/** True while the pane has skipped layout and not yet caught up. */
export function layoutBehind(id: string): boolean {
    return behind().has(id);
}

function drainBehind(): void {
    cancelDrain?.();
    cancelDrain = drainWhenIdle(() => {
        const [next] = untrack(behind);
        if (next == null) return false;
        const rest = new Set(untrack(behind));
        rest.delete(next);
        setBehind(rest);
        return rest.size > 0;
    });
}

let installedOn: Window | null = null;

/** Start tracking `resize` events on `target`. Idempotent. */
export function installWindowResizeTracking(target: Window = window): void {
    if (installedOn === target) return;
    installedOn?.removeEventListener("resize", noteWindowResize);
    target.addEventListener("resize", noteWindowResize);
    installedOn = target;
}

/** Test hook: forget all state. */
export function resetWindowResizeForTests(): void {
    if (settleTimer != null) clearTimeout(settleTimer);
    settleTimer = null;
    setResizing(false);
    cancelDrain?.();
    cancelDrain = null;
    setBehind(new Set<string>());
    installedOn?.removeEventListener("resize", noteWindowResize);
    installedOn = null;
}

/**
 * Run `step` once per idle period until it returns false, so work deferred
 * during a resize catches up without one long frame. Returns a cancel.
 */
export function drainWhenIdle(step: () => boolean, timeoutMs = 250): () => void {
    let cancelled = false;
    let handle: number | ReturnType<typeof setTimeout> | null = null;
    const ric = typeof requestIdleCallback === "function" ? requestIdleCallback : null;
    const schedule = () => {
        if (cancelled) return;
        handle = ric ? ric(tick, { timeout: timeoutMs }) : setTimeout(tick, 16);
    };
    const tick = () => {
        handle = null;
        if (cancelled) return;
        if (step()) schedule();
    };
    schedule();
    return () => {
        cancelled = true;
        if (handle == null) return;
        if (ric && typeof handle === "number") cancelIdleCallback(handle);
        else clearTimeout(handle as ReturnType<typeof setTimeout>);
    };
}
