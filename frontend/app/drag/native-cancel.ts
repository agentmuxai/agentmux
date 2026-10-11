// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Escape during a drag, on every platform.
 *
 * The page doesn't see the key while the OS runs the drag: Windows runs it in
 * its own OLE loop, and on macOS and Linux Chromium's native drag holds back
 * page `keydown` too (see the macOS note in tab-tearoff-events.ts). So nothing
 * marks the drag escaped, and a break-off still opens a window. The drag does
 * leave a trace, though. Escape ends it at once, while the left button is
 * still held, and the page gets a `dragend` with no drop effect, as it does
 * for a release over nothing. Two things tell them apart:
 *
 * - the button: still down after Escape, up after a release. The host reads it
 *   without any permission (`GetAsyncKeyState`, `CGEventSourceButtonState`, an
 *   X11 pointer query);
 * - the key itself: once the drag is over, the page does get Escape's key
 *   events. On Linux (native Wayland, where the button can't be read) the
 *   `keyup` arrives about 10 ms after the `dragend`.
 *
 * On macOS the tear-off hook's tap also reports Escape when it has the
 * permission it needs; this works without it, and for pane tabs, which have
 * no hook.
 *
 * A release the page can't see (over another app) never gets this far:
 * CrossWindowDragMonitor.win32's fallback handles it, and asks the button
 * state itself.
 */

import { getApi } from "@/store/global";
import { Logger } from "@/util/logger";

/** How long after a bare `dragend` an Escape key event still counts as the
 *  cancel that ended it (Linux measured 9–13 ms). A release waits this long
 *  before it acts. */
export const ESCAPE_AFTER_DRAGEND_MS = 60;

/** An Escape the page saw this soon before the `dragend` counts too. */
const ESCAPE_BEFORE_DRAGEND_MS = 50;

let lastEscapeAt = Number.NEGATIVE_INFINITY;

/** Escape's key events, whenever they come: the `keyup` that follows a
 *  cancel can land while the button query is still in flight. */
function noteEscape(e: KeyboardEvent): void {
    if (e.key === "Escape") lastEscapeAt = performance.now();
}
if (typeof window !== "undefined") {
    window.addEventListener("keydown", noteEscape, true);
    window.addEventListener("keyup", noteEscape, true);
}

/** The decision, apart from the OS query: no drop effect, button still down. */
export function isCancelledDragEnd(dropEffect: string | undefined, buttonStillDown: boolean): boolean {
    return dropEffect === "none" && buttonStillDown;
}

/** Whether an Escape key event came near the `dragend` at `endedAt`: from
 *  just before it until {@link ESCAPE_AFTER_DRAGEND_MS} after. Returns as soon
 *  as one is seen. */
async function escapeKeyNear(endedAt: number): Promise<boolean> {
    const since = endedAt - ESCAPE_BEFORE_DRAGEND_MS;
    const until = endedAt + ESCAPE_AFTER_DRAGEND_MS;
    while (lastEscapeAt < since && performance.now() < until) {
        await new Promise((resolve) => setTimeout(resolve, 10));
    }
    return lastEscapeAt >= since;
}

/**
 * Whether the drag that just ended (`dragend`'s drop effect) was cancelled with
 * Escape. Call it straight from the `dragend` (or the drop it ends in), before
 * other awaits, so the time it measures from is the drag's end.
 */
export async function endedByEscape(dropEffect: string | undefined): Promise<boolean> {
    if (dropEffect !== "none") return false;
    const endedAt = performance.now();
    let buttonStillDown = false;
    try {
        buttonStillDown = await getApi().getMouseButtonState();
    } catch (e) {
        // A host that can't read the button (the web, a Wayland session
        // without X): the key decides.
        Logger.warn("dnd:cross", "get_mouse_button_state failed after dragend", { error: String(e) });
    }
    const escapeSeen = buttonStillDown ? false : await escapeKeyNear(endedAt);
    const cancelled = isCancelledDragEnd(dropEffect, buttonStillDown) || escapeSeen;
    Logger.info("dnd:cross", "dragend with no drop effect", { buttonStillDown, escapeSeen, cancelled });
    return cancelled;
}

/** Forget any Escape seen so far, so tests start clean. */
export function forgetEscapeForTest(): void {
    lastEscapeAt = Number.NEGATIVE_INFINITY;
}
