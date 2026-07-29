// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tab-strip reorder drag-and-drop, split out of tabbar.tsx: the
// pragmatic-drag-and-drop monitorForElements/dropTargetForElements wiring,
// mouse-wheel horizontal scroll, and the Windows tear-off-cursor
// workaround. Module-level drag state (insertionPoint, bouncingTabId,
// dragActivatedTabIds, …) lives in tabbar-dnd.ts — this file reads/writes
// that shared state rather than duplicating it.

import { onCleanup, onMount } from "solid-js";
import { fireAndForget } from "@/util/util";
import { isWindows } from "@/util/platformutil";
import { monitorForElements, dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { getLayoutModelForTabById, tileItemType } from "@/layout/index";
import { WorkspaceService } from "../store/services";
import {
    tabItemType,
    insertionPoint,
    setInsertionPoint,
    setBouncingTabId,
    computeInsertionPoint,
    InsertionPoint,
    dragActivatedTabIds,
    globalDragTabId,
    dragEscaped,
    setDragEscaped,
    TEAR_PAST_PX,
    cleanupTileDragState,
} from "./tabbar-dnd";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import type { TearOffTabAtReleaseFn } from "./tab-tearoff-rpc";
import { Logger } from "@/util/logger";

/**
 * Execute the reorder described by the insertion point.
 * All drop logic lives here — droppable-tab.tsx is visual-only. Exported so
 * droppable-tab.tsx's Windows native-pointer-drag tracker
 * (SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28.md) can call the exact same
 * commit logic the pragmatic-dnd monitor below uses on non-Windows.
 */
export function executeReorder(
    ip: InsertionPoint,
    draggedTabId: string,
    tabs: string[],
    wsId: string
): void {
    let insertIdx: number;
    if (ip.beforeTabId === null) {
        insertIdx = 0;
    } else if (ip.afterTabId === null) {
        insertIdx = tabs.length;
    } else {
        insertIdx = tabs.indexOf(ip.afterTabId);
    }

    const sourceIdx = tabs.indexOf(draggedTabId);
    if (sourceIdx < 0 || insertIdx < 0) return;
    // Adjust for element removal shifting indices
    const finalIdx = sourceIdx < insertIdx ? insertIdx - 1 : insertIdx;
    fireAndForget(async () => {
        try {
            await WorkspaceService.ReorderTab(wsId, draggedTabId, finalIdx);
        } catch (e) {
            Logger.error("dnd", "tab-reorder failed", { tabId: draggedTabId, finalIdx, error: String(e) });
        }
    });
}

/**
 * Wires up in-strip tab reordering (pragmatic-dnd monitor + drop target),
 * the pane(tile)-drag-over-strip cleanup that shares the same monitor
 * infrastructure, the Windows tear-off-cursor workaround, and mouse-wheel
 * horizontal scrolling. Must be called during SolidJS component setup
 * (uses onMount/onCleanup internally).
 */
export function useTabDragAndDrop(
    refs: { tabBarScrollRef: () => HTMLDivElement },
    workspace: () => Workspace,
    tabIds: () => string[],
    tearOffTabAtRelease: TearOffTabAtReleaseFn,
): void {
    const { tabBarScrollRef } = refs;

    onMount(() => {
        // Tab-item drag detection (in-strip reorder + commit-on-release
        // tear-off) is pragmatic-dnd/HTML5-driven and stays exactly as it
        // was — but ONLY on macOS/Linux. On Windows, droppable-tab.tsx now
        // drives the equivalent gesture from a raw Pointer Events state
        // machine (attachNativePointerDragTracker,
        // SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28.md) that never starts
        // an HTML5 drag session in the first place, so none of this block
        // (including the old Windows-only dropEffect="copy" tear-off-cursor
        // workaround it used to contain — deleted, nothing left to fight)
        // ever runs there. `executeReorder`/`computeInsertionPoint` are
        // still shared, just invoked directly by the tracker instead of via
        // this monitor's callbacks.
        if (!isWindows()) {
            // Escape-to-abort: pragmatic-drag-and-drop's HTML5 drag session
            // doesn't cancel itself on Escape (unlike a native OS drag loop,
            // the web DnD spec gives browsers no obligation to honor it), so
            // without this the tab drag simply continues regardless of
            // Escape and still tears off / reorders normally on release.
            // Mouse events are unreliable during an active HTML5 drag, but
            // keyboard events are still delivered to the page normally, so a
            // plain `keydown` listener works. Sets a flag `onDrop` below
            // checks before deciding tear-off vs. reorder, rather than
            // trying to interrupt the drag itself (there's no such API for
            // HTML5 DnD).
            const onDragEscape = (e: KeyboardEvent) => {
                if (globalDragTabId == null) return; // not a tab drag
                if (e.key !== "Escape") return;
                setDragEscaped(true);
            };
            window.addEventListener("keydown", onDragEscape, true);
            onCleanup(() => window.removeEventListener("keydown", onDragEscape, true));

            const cleanup = monitorForElements({
                canMonitor: ({ source }) => source.data.type === tabItemType,

                // Always compute insertion point from cursor position — drives the gap animation on all tabs
                onDrag: ({ location }) => {
                    // Tear-off is committed on RELEASE now (see onDrop), not
                    // mid-drag — dragging down no longer eagerly spawns a
                    // window that follows the cursor. So onDrag only tracks the
                    // insertion point that drives the reorder gap animation.
                    setInsertionPoint(computeInsertionPoint(location.current.input.clientX));
                },

                onDrop: ({ source, location }) => {
                    // Escape was pressed at some point during this drag (see
                    // the keydown listener above) — abort the WHOLE operation:
                    // no reorder, no tear-off. The tab was never actually moved
                    // (only the insertion-point preview did), so clearing that
                    // and the cross-window payload is enough to fully restore
                    // the pre-drag state; nothing to undo.
                    if (dragEscaped) {
                        setDragEscaped(false);
                        setCurrentDragPayload(null);
                        setInsertionPoint(null);
                        Logger.info("dnd", "tab drag aborted via Escape", {
                            draggedTabId: source.data.tabId,
                        });
                        return;
                    }

                    const ip = insertionPoint();
                    const draggedTabId = source.data.tabId as string;

                    // `insertionPoint` reflects the last cursor X, so it can be
                    // non-null even when the user has dragged BELOW the tab bar
                    // for a tear-off. Pragmatic-dnd registers no drop target for
                    // the bar (insertion is purely X-driven), so we hit-test the
                    // cursor against the strip's bounding rect ourselves to tell
                    // "reorder inside the bar" from "tear-off below it".
                    const input = location.current.input;
                    const rect = tabBarScrollRef()?.getBoundingClientRect();
                    const dropInsideBar =
                        rect != null &&
                        input.clientY >= rect.top && input.clientY <= rect.bottom &&
                        input.clientX >= rect.left && input.clientX <= rect.right;

                    // Commit-on-release tear-off: the tab was released BELOW the
                    // strip (dragged down into the window body) and let go.
                    // Spawn the new window at the release point NOW — deliberately
                    // not mid-drag, so nothing detaches until the user releases
                    // (the behaviour they expect). Lone tabs never tear (tearing
                    // the only tab would just trade one single-tab window for
                    // another and strand the source); their cross-window exit is
                    // the host mouse-hook remount.
                    const releasedBelowStrip =
                        rect != null && input.clientY > rect.bottom + TEAR_PAST_PX;
                    if (
                        !dropInsideBar &&
                        releasedBelowStrip &&
                        draggedTabId != null &&
                        tabIds().length > 1
                    ) {
                        // Clear the payload SYNCHRONOUSLY (before the async
                        // tear-off) so CrossWindowDragMonitor's dragend handler —
                        // which may fire for the same gesture — sees no payload
                        // and doesn't double-process this tear.
                        setCurrentDragPayload(null);
                        tearOffTabAtRelease(draggedTabId, input);
                        setInsertionPoint(null);
                        return;
                    }

                    const willReorder = dropInsideBar && ip != null && draggedTabId != null;

                    if (willReorder || location.current.dropTargets.length > 0) {
                        setCurrentDragPayload(null);
                    }

                    if (willReorder) {
                        const tabs = tabIds();
                        const wsId = workspace()?.oid;

                        executeReorder(ip, draggedTabId, tabs, wsId);

                        // Trigger bounce on the dragged tab at its new position
                        setBouncingTabId(draggedTabId);
                        setTimeout(() => setBouncingTabId(null), 400);

                        Logger.info("dnd", "tab drop", {
                            draggedTabId,
                            beforeTabId: ip.beforeTabId,
                            afterTabId: ip.afterTabId,
                            workspaceId: wsId,
                        });
                    }

                    setInsertionPoint(null);
                },
            });
            onCleanup(cleanup);
        }

        // Pane (tile) drags over the tab bar (SPEC_PANE_DRAG_TO_TAB_2026_07_10.md,
        // spring-loaded-tabs revision). The interactive pieces live in
        // DroppableTab (per-tab drop target: flash → dwell → switch active
        // tab; drop-on-tab-button = append) and TileLayout's overlay (in-tab
        // ghost + redock on drop, once the spring switch has made the target
        // tab visible). Two things remain at the bar level:
        //
        // 1. The strip itself must be a drop TARGET for tiles — without one,
        //    the browser shows the not-allowed cursor over the bar's empty
        //    areas and, worse, an unconsumed release leaves
        //    `currentDragPayload` set for CrossWindowDragMonitor's dragend
        //    listener, which misreads it as a tear-off and spawns an
        //    unwanted floating window (TileLayout's own onDrop deliberately
        //    never clears the payload — it fires for out-of-window drops too).
        const cleanupStripDrop = dropTargetForElements({
            element: tabBarScrollRef(),
            // Accept BOTH tile drags (pane→tab) and tab drags (reorder).
            // For a tab reorder the actual work is done by the
            // monitorForElements above (X-driven insertion); this drop
            // target exists only so pragmatic-dnd calls preventDefault on
            // dragover while the tab is over the strip, which sets
            // dropEffect="move" and kills the Windows circle-slash/no-drop
            // cursor. Without a tab-accepting drop target, nothing calls
            // preventDefault on Windows (preventUnhandled is gated off there
            // to protect the SC_MOVE tear-off path — see droppable-tab.tsx),
            // so Chromium showed the not-allowed cursor for every reorder.
            // Scoped to the strip: dragging a tab OUT of the strip (tear-off)
            // leaves this target and is unaffected. onDrop clears the payload
            // for both kinds (harmless double-clear for tabs — the monitor's
            // onDrop already clears it on a real reorder).
            canDrop: ({ source }) =>
                source.data.type === tileItemType || source.data.type === tabItemType,
            onDrop: () => {
                setCurrentDragPayload(null);
            },
        });

        // 2. End-of-drag cleanup, wherever the release happens (a monitor's
        //    onDrop fires for cancelled drags too): stop any pending spring
        //    switch, kill the hover flash, drop any un-consumed cross-tab
        //    record, and deactivate the overlay of every tab the drag
        //    spring-switched through (their activeDrag was set by
        //    DroppableTab; TileLayout's own cleanup only covers the SOURCE
        //    tab's model).
        //
        //    A stuck activeDrag is a DEAD TAB — the overlay-container sits
        //    over the entire tile area with pointer-events:auto and eats
        //    every click — so the cleanup runs from three layers:
        //    a) pragmatic's monitor onDrop (normal path),
        //    b) a window dragend listener (pragmatic's dispatch can be
        //       skipped on Win11 swallowed-drag paths — same rationale as
        //       TileLayout's own resetDragState safety net),
        //    c) a capture-phase pointerdown listener: a pointerdown cannot
        //       happen mid-drag (the button is held), so any pointerdown
        //       with spring-activated tabs still recorded means the drag
        //       ended without (a) or (b) firing — clean up before the
        //       stuck overlay swallows the click's target.
        //
        // Shared with TileLayout.win32.tsx's native pointer-drag tracker
        // (Windows pane source — SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28
        // §3.5), which has no pragmatic-dnd monitor to dispatch this from —
        // see cleanupTileDragState's own doc comment in tabbar-dnd.ts.
        const cleanupTileMonitor = monitorForElements({
            canMonitor: ({ source }) => source.data.type === tileItemType,
            onDrop: cleanupTileDragState,
        });
        const onWindowDragEnd = () => {
            if (dragActivatedTabIds.size > 0) cleanupTileDragState();
        };
        const onWindowPointerDown = () => {
            if (dragActivatedTabIds.size > 0) {
                // Reaching this net means the drag ended UNOBSERVED (no
                // monitor onDrop, no window dragend) — log loudly with
                // each tab's overlay state so field diags can pinpoint
                // what wedged. (SPEC_PANE_DRAG_TO_TAB addendum A2.)
                Logger.warn("dnd", "pointerdown net fired — drag ended unobserved", {
                    activated: [...dragActivatedTabIds],
                    overlays: tabIds().map((id) => ({
                        tabId: id,
                        activeDrag: getLayoutModelForTabById(id)?.activeDrag() ?? null,
                    })),
                });
                cleanupTileDragState();
            }
        };
        window.addEventListener("dragend", onWindowDragEnd);
        window.addEventListener("pointerdown", onWindowPointerDown, true);

        onCleanup(() => {
            cleanupStripDrop();
            cleanupTileMonitor();
            window.removeEventListener("dragend", onWindowDragEnd);
            window.removeEventListener("pointerdown", onWindowPointerDown, true);
        });
    });

    // Mouse-wheel hover over the strip → horizontal scroll. Ctrl+wheel
    // is reserved for the app-wide zoom (see AppZoomHandler in app.tsx),
    // so this only kicks in for unmodified scrolls. We forward whichever
    // delta dominates: `deltaX` if a horizontal-aware mouse / touchpad is
    // already producing it, otherwise `deltaY` so a normal vertical wheel
    // still scrolls the strip horizontally — matches Chrome / Edge tab
    // strip behaviour. preventDefault so the page doesn't try to scroll.
    const handleStripWheel = (e: WheelEvent) => {
        if (e.ctrlKey || e.metaKey) return;
        const delta = e.deltaX !== 0 ? e.deltaX : e.deltaY;
        if (delta === 0) return;
        const scrollEl = tabBarScrollRef();
        if (!scrollEl) return;
        e.preventDefault();
        scrollEl.scrollLeft += delta;
    };

    onMount(() => {
        const scrollEl = tabBarScrollRef();
        if (!scrollEl) return;
        // `passive: false` is required so preventDefault works.
        scrollEl.addEventListener("wheel", handleStripWheel, { passive: false });
        onCleanup(() => tabBarScrollRef()?.removeEventListener("wheel", handleStripWheel));
    });
}
