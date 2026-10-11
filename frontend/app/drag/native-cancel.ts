// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Escape during a drag, on Windows.
 *
 * The page never sees the key: Windows runs the drag in its own OLE loop, and
 * a DOM `keydown` listener doesn't fire for its duration (see the macOS note in
 * tab-tearoff-events.ts), so nothing marks the drag escaped and a break-off
 * still opens a window. The loop does leave a trace, though. It ends on a
 * release (button up) or on Escape (button still down), and the page gets a
 * `dragend` either way, with no drop effect. So a `dragend` with no effect
 * while the left button is still held was cancelled with Escape.
 *
 * A release the page can't see (over another app) never gets this far:
 * CrossWindowDragMonitor.win32's fallback handles it, and asks the button
 * state itself.
 */

import { getApi } from "@/store/global";
import { Logger } from "@/util/logger";
import { isWindows } from "@/util/platformutil";

/** The decision, apart from the OS query: no drop effect, button still down. */
export function isCancelledDragEnd(dropEffect: string | undefined, buttonStillDown: boolean): boolean {
    return dropEffect === "none" && buttonStillDown;
}

/**
 * Whether the drag that just ended (`dragend`'s drop effect) was cancelled with
 * Escape. Windows only; elsewhere the page sees the key itself.
 */
export async function endedByEscape(dropEffect: string | undefined): Promise<boolean> {
    if (!isWindows() || dropEffect !== "none") return false;
    let buttonStillDown = false;
    try {
        buttonStillDown = await getApi().getMouseButtonState();
    } catch (e) {
        // Can't tell: treat it as a release, the same as before this check.
        Logger.warn("dnd:cross", "get_mouse_button_state failed after dragend", { error: String(e) });
    }
    const cancelled = isCancelledDragEnd(dropEffect, buttonStillDown);
    Logger.info("dnd:cross", "dragend with no drop effect", { buttonStillDown, cancelled });
    return cancelled;
}
