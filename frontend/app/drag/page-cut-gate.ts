// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a browser page is still on screen at the start of an in-app drag. A browser pane's page is a native
 * window above the DOM, which no CSS moves. For the length of a drag each one is cut away behind a picture of
 * itself (the browser view's drag catcher), and from then on its tile is plain DOM that the live reflow can
 * move like any other (liveReflow.ts). Until the cut has landed the page still stands where it was, so the
 * reflow shows no drop: a tile that moved first would leave its page behind.
 *
 * The catcher takes its picture first (use-drag-snapshot.ts) and gives up waiting after half a second, so a
 * cut is never far off; one that never reports stops holding the reflow back after `PAGE_CUT_MAX_MS`. No
 * dependencies, so the layout can read it without the browser modules (as pane-capture-gate.ts).
 */

import { createSignal } from "solid-js";

/** The longest one page holds the drag's reflow back. */
export const PAGE_CUT_MAX_MS = 800;

const [pending, setPending] = createSignal(0);

/** Whether a page is still to be cut away for the drag in progress. Reactive. */
export const pageCutsPending = (): boolean => pending() > 0;

/** A page is about to be cut away: returns what to call once it is, or once its catcher is gone. Safe to repeat. */
export function beginPageCut(): () => void {
    let open = true;
    setPending((n) => n + 1);
    const done = () => {
        if (!open) return;
        open = false;
        clearTimeout(timer);
        setPending((n) => Math.max(0, n - 1));
    };
    const timer = setTimeout(done, PAGE_CUT_MAX_MS);
    return done;
}

/** Test hook. */
export function resetPageCutGateForTests(): void {
    setPending(0);
}
