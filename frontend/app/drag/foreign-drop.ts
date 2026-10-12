// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A window tab dropped onto another AgentMux window, on Linux.
 *
 * The host can't tell which window a release landed on: native Wayland gives
 * no client its windows' screen positions, and off Windows there's no window
 * hit test at all. The window the tab was dropped on can, though: it gets the
 * drop. So the dragged tab carries its details ({@link WINDOW_TAB_MIME}), and a
 * window that sees them from another window accepts a drop on its tab strip
 * and merges the tab itself, the same way the Windows and macOS hooks'
 * `tabdrag:merge-direct` does (a window's last tab restores into it and
 * closes the emptied window). Anywhere else in the window it isn't accepted,
 * and the tab tears off, as in Chrome.
 *
 * The source must not also act on the release. It can't know for sure that
 * another window took it: on X11 a release over the desktop also ends with
 * drop effect "move". So it defers whenever the drag ended with "move" and no
 * `drop` in its own window ({@link maybeDroppedInAnotherWindow}), and its
 * cross-window drag-end asks the host for the target. The window that merged
 * the tab has claimed the drop (`claim_cross_drag_target`), so the host names
 * it; with no claim the source tears off. A lone tab has no such drag-end, so
 * nothing claims it.
 */

import { getApi } from "@/store/global";
import { Logger } from "@/util/logger";
import { isLinux } from "@/util/platformutil";

/** The type a dragged window tab's details travel under, so another window can tell it from other drags. */
export const WINDOW_TAB_MIME = "application/x-agentmux-window-tab";

/** What another window needs to merge a dropped window tab. */
export interface WindowTabDragData {
    tabId: string;
    fromWsId: string;
    sourceWindowLabel: string;
    /** The source window's only tab: merging it closes that window. */
    isLastTab: boolean;
}

/** A window tab's details as drag data, for `getInitialDataForExternal`. */
export function windowTabDragData(data: WindowTabDragData): Record<string, string> {
    return { [WINDOW_TAB_MIME]: JSON.stringify(data) };
}

function parseWindowTabDragData(raw: string | undefined): WindowTabDragData | null {
    try {
        const d = JSON.parse(raw ?? "");
        const ok = typeof d?.tabId === "string" && typeof d?.fromWsId === "string" && typeof d?.sourceWindowLabel === "string";
        return ok ? { tabId: d.tabId, fromWsId: d.fromWsId, sourceWindowLabel: d.sourceWindowLabel, isLastTab: d.isLastTab === true } : null;
    } catch {
        return null;
    }
}

/** Whether this window saw a `drop` during the current drag (a drag that
 *  started here resets it). */
let droppedHere = false;

/** Whether the drag under way started in this window: from its `dragstart`
 *  to its `dragend`. Not the drag session, which a drop handler ends before
 *  the window-level `drop` listener below runs. */
let dragStartedHere = false;

if (typeof window !== "undefined") {
    window.addEventListener("dragstart", () => {
        droppedHere = false;
        dragStartedHere = true;
    }, true);
    window.addEventListener("drop", () => (droppedHere = true), true);
    window.addEventListener("dragend", () => (dragStartedHere = false), true);
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
    return !dragStartedHere && (e.dataTransfer?.types ?? []).includes(WINDOW_TAB_MIME);
}

/** Where a window accepts a window tab from another window, and what it does with one. */
export interface WindowTabDropTarget {
    /** Whether this point takes the drop (the tab strip). */
    accepts: (e: DragEvent) => boolean;
    merge: (data: WindowTabDragData, e: DragEvent) => void;
}

/**
 * Linux: accept window tabs dragged in from other windows where `target`
 * accepts them. Each one dropped there goes to `target.merge`, and is claimed
 * with the host so its source doesn't also tear it off. Returns the disposer.
 */
export function acceptWindowTabsFromOtherWindows(windowLabel: string, target: WindowTabDropTarget): () => void {
    if (!isLinux()) return () => {};
    const onDragOver = (e: DragEvent) => {
        if (!isWindowTabFromElsewhere(e) || !target.accepts(e)) return;
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    };
    const onDrop = (e: DragEvent) => {
        if (!isWindowTabFromElsewhere(e) || !target.accepts(e)) return;
        e.preventDefault();
        const data = parseWindowTabDragData(e.dataTransfer?.getData(WINDOW_TAB_MIME));
        if (!data) {
            Logger.warn("dnd:cross", "window tab dropped from another window without readable details", { windowLabel });
            return;
        }
        Logger.info("dnd:cross", "window tab dropped here from another window", { windowLabel, tabId: data.tabId, isLastTab: data.isLastTab });
        if (!data.isLastTab) {
            getApi()
                .claimCrossDragTarget?.(windowLabel)
                .catch((err) => Logger.warn("dnd:cross", "claim_cross_drag_target failed", { error: String(err) }));
        }
        target.merge(data, e);
    };
    window.addEventListener("dragover", onDragOver);
    window.addEventListener("drop", onDrop);
    return () => {
        window.removeEventListener("dragover", onDragOver);
        window.removeEventListener("drop", onDrop);
    };
}
