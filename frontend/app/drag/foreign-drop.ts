// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A window tab dropped onto another AgentMux window, on Linux.
 *
 * The host can't tell which window a release landed on: native Wayland gives
 * no client its windows' screen positions, and off Windows there's no window
 * hit test at all. The window the tab was dropped on can, though: it gets the
 * drop. So the dragged tab carries a marker ({@link WINDOW_TAB_MIME}), a
 * window that sees it from another window accepts the drop, and on `drop`
 * tells the host it took it (`claim_cross_drag_target`). The source's
 * cross-window drag-end then asks the host for the target, as it always did,
 * and the existing cross-window pipeline moves the tab.
 *
 * The source has to hand the release to that path rather than tear the tab
 * off itself. It can't know for sure that another window took it: on X11 a
 * release over the desktop also ends with drop effect "move". So it defers
 * whenever the drag ended with "move" and no `drop` in its own window
 * ({@link maybeDroppedInAnotherWindow}); the cross-window path tears off if
 * no window claims the drop.
 */

import { getApi } from "@/store/global";
import { Logger } from "@/util/logger";
import { isLinux } from "@/util/platformutil";
import { session } from "./drag-session";

/** The marker a dragged window tab carries, so another window can tell it from other drags. */
export const WINDOW_TAB_MIME = "application/x-agentmux-window-tab";

/** Whether this window saw a `drop` during the current drag (a drag that
 *  started here resets it). */
let droppedHere = false;

if (typeof window !== "undefined") {
    window.addEventListener("dragstart", () => (droppedHere = false), true);
    window.addEventListener("drop", () => (droppedHere = true), true);
}

/**
 * Linux: whether the drag that just ended (its `dragend` drop effect) may have
 * been dropped onto another AgentMux window. Then the release isn't this
 * window's to act on; the cross-window drag-end decides.
 */
export function maybeDroppedInAnotherWindow(dropEffect: string | undefined): boolean {
    return isLinux() && dropEffect === "move" && !droppedHere;
}

/** A window-tab drag that started in another window. */
function isWindowTabFromElsewhere(e: DragEvent): boolean {
    return session() == null && (e.dataTransfer?.types ?? []).includes(WINDOW_TAB_MIME);
}

/**
 * Linux: accept window tabs dragged in from other windows, and claim each one
 * dropped here with the host. Returns the disposer.
 */
export function acceptWindowTabsFromOtherWindows(windowLabel: string): () => void {
    if (!isLinux()) return () => {};
    const onDragOver = (e: DragEvent) => {
        if (!isWindowTabFromElsewhere(e)) return;
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    };
    const onDrop = (e: DragEvent) => {
        if (!isWindowTabFromElsewhere(e)) return;
        e.preventDefault();
        Logger.info("dnd:cross", "window tab dropped here from another window; claiming the drop", { windowLabel });
        getApi()
            .claimCrossDragTarget?.(windowLabel)
            .catch((err) => Logger.warn("dnd:cross", "claim_cross_drag_target failed", { error: String(err) }));
    };
    window.addEventListener("dragover", onDragOver);
    window.addEventListener("drop", onDrop);
    return () => {
        window.removeEventListener("dragover", onDragOver);
        window.removeEventListener("drop", onDrop);
    };
}
