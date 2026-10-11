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
 * for a release over nothing. They are told apart by:
 *
 * - the button, wherever the host can read it: still down after Escape, up
 *   after a release (`GetAsyncKeyState`, `CGEventSourceButtonState`, an X11
 *   pointer query; none of them needs a permission);
 * - the key, where it can't (native Wayland): once the drag is over the page
 *   gets Escape's `keyup`, when the key is let go.
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

/** Where the button can't be read: how long after a bare `dragend` the
 *  Escape that ended it may still be let go (its `keyup`), and so how long a
 *  release waits before it acts. A held key is let go well within this. */
export const ESCAPE_KEY_WAIT_MS = 300;

/** An Escape the page saw this soon before the `dragend` counts too. */
const ESCAPE_BEFORE_DRAGEND_MS = 50;

/** When the page last saw one of Escape's key events. */
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

/** Whether one of Escape's key events falls between just before the
 *  `dragend` at `endedAt` and {@link ESCAPE_KEY_WAIT_MS} after it. Returns as
 *  soon as one does. */
async function escapeKeyNear(endedAt: number): Promise<boolean> {
    const since = endedAt - ESCAPE_BEFORE_DRAGEND_MS;
    const until = endedAt + ESCAPE_KEY_WAIT_MS;
    const inWindow = () => lastEscapeAt >= since && lastEscapeAt <= until;
    while (!inWindow() && performance.now() < until) {
        await new Promise((resolve) => setTimeout(resolve, 10));
    }
    return inWindow();
}

/**
 * Whether the drag that just ended (`dragend`'s drop effect) was cancelled with
 * Escape. Call it straight from the `dragend` (or the drop it ends in), before
 * other awaits, so the time it measures from is the drag's end.
 */
export async function endedByEscape(dropEffect: string | undefined): Promise<boolean> {
    if (dropEffect !== "none") return false;
    const endedAt = performance.now();
    let buttonStillDown: boolean | null = null;
    try {
        buttonStillDown = await getApi().getMouseButtonState();
    } catch (e) {
        // A host that can't read the button (native Wayland, the web): the
        // key decides.
        Logger.info("dnd:cross", "button state can't be read after dragend", { error: String(e) });
    }
    const escapeSeen = buttonStillDown == null ? await escapeKeyNear(endedAt) : null;
    const cancelled = buttonStillDown != null ? isCancelledDragEnd(dropEffect, buttonStillDown) : escapeSeen === true;
    Logger.info("dnd:cross", "dragend with no drop effect", { buttonStillDown, escapeSeen, cancelled });
    return cancelled;
}

/** Forget any Escape seen so far, so tests start clean. */
export function forgetEscapeForTest(): void {
    lastEscapeAt = Number.NEGATIVE_INFINITY;
}
