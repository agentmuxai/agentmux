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
import { hostHas } from "@/app/host/host-caps";
import { isWindows } from "@/util/platformutil";
import { monitorForElements, dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { clearCrossTabDrop, getLayoutModelForTabById } from "@/layout/index";
import { tabItemType, tileItemType } from "@/app/drag/drag-types";
import { markEscaped, onSessionEnded, session } from "@/app/drag/drag-session";
import { onWindowDrag } from "@/app/drag/window-drag-events";
import { endTileDrag } from "@/layout/lib/tile-drag";
import { pruneDanglingLeaves } from "@/layout/lib/layoutPersistence";
import { WorkspaceService } from "../store/services";
import {
    insertionPoint,
    setInsertionPoint,
    setBouncingTabId,
    computeInsertionPoint,
    InsertionPoint,
    dragActivatedTabIds,
    draggedWindowTabId,
    endWindowTabDrag,
    setHoveredDropTabId,
    decideTabRelease,
} from "./tabbar-dnd";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import type { TearOffTabAtReleaseFn } from "./tab-tearoff-rpc";
import { Logger } from "@/util/logger";

/**
 * Execute the reorder described by the insertion point.
 * All drop logic lives here — droppable-tab.tsx is visual-only.
 */
function executeReorder(
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
        // Tear-off cursor (Windows). During a tab drag, the tear-off zone
        // (cursor dragged OUTSIDE the strip) has no pragmatic drop target,
        // and preventUnhandled is gated off on Windows — its SC_MOVE
        // tear-off relies on the native window-move handshake, not the
        // HTML5 snapback. pragmatic only calls preventDefault / sets
        // dropEffect while over a drop target (see lifecycle-manager:
        // `innerMost != null`), so out in the tear-off zone Chromium falls
        // back to the no-drop circle-slash cursor — which reads as "you
        // can't drop this", the opposite of the truth: releasing below the
        // strip spawns a NEW window.
        //
        // Fill that gap ourselves: while a tab drag is in flight and the
        // cursor is outside the strip, preventDefault + set dropEffect to
        // "copy" so Chromium paints the "plus" cursor (the universal
        // "create new" affordance). Over the strip we do nothing, leaving
        // the move cursor to the strip's own drop target.
        //
        // The listener is installed ONCE here (not in the monitor's
        // onDragStart) and gated on the window-tab drag session, which
        // spans the whole tab drag. This
        // keeps it alive across HMR (which does not re-run a monitor's
        // onDragStart) and independent of pragmatic's monitor dispatch.
        // macOS/Linux already dodge the circle-slash via preventUnhandled,
        // so this is Windows-only.
        if (isWindows()) {
            const onTearOffDragOver = (e: DragEvent) => {
                if (draggedWindowTabId() == null) return; // not a tab drag
                const rect = tabBarScrollRef()?.getBoundingClientRect();
                const overStrip =
                    rect != null &&
                    e.clientX >= rect.left && e.clientX <= rect.right &&
                    e.clientY >= rect.top && e.clientY <= rect.bottom;
                if (overStrip) return; // strip drop target owns the cursor here
                e.preventDefault();
                if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
            };
            onCleanup(onWindowDrag({ kinds: ["window-tab"], over: onTearOffDragOver }));
        }

        // Escape-to-abort (cross-platform): pragmatic-drag-and-drop's HTML5
        // drag session doesn't cancel itself on Escape (unlike a native OS
        // drag loop, the web DnD spec gives browsers no obligation to honor
        // it), so without this the tab drag simply continues regardless of
        // Escape and still tears off / reorders normally on release. Mouse
        // events are unreliable during an active HTML5 drag, but keyboard
        // events are still delivered to the page normally, so a plain
        // `keydown` listener works — gated the same way as the Windows
        // tear-off-cursor listener above. Sets a flag `onDrop` below checks
        // before deciding tear-off vs. reorder, rather than trying to
        // interrupt the drag itself (there's no such API for HTML5 DnD).
        const onDragEscape = (e: KeyboardEvent) => {
            if (draggedWindowTabId() == null) return; // not a tab drag
            if (e.key !== "Escape") return;
            markEscaped();
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
                // Every window-tab release reaches this monitor, after the
                // source's onDrop (pragmatic: source, targets, monitors).
                const escaped = session()?.escaped ?? false;
                endWindowTabDrag("drop");

                const ip = insertionPoint();
                const draggedTabId = source.data.tabId as string;
                const input = location.current.input;
                // Pragmatic-dnd registers no drop target for the bar
                // (insertion is purely X-driven), so the strip's rect tells
                // "reorder inside the bar" from "tear-off below it".
                const release = decideTabRelease({
                    escaped,
                    ip,
                    input,
                    stripRect: tabBarScrollRef()?.getBoundingClientRect() ?? null,
                    tabCount: tabIds().length,
                    draggedTabId,
                    canTearOff: hostHas("tearOff"),
                });

                // Escape was pressed at some point during this drag (see
                // the keydown listener above) — abort the WHOLE operation:
                // no reorder, no tear-off. The tab was never actually moved
                // (only the insertion-point preview did), so clearing that
                // and the cross-window payload is enough to fully restore
                // the pre-drag state; nothing to undo.
                if (release === "abort") {
                    setCurrentDragPayload(null);
                    setInsertionPoint(null);
                    Logger.info("dnd", "tab drag aborted via Escape", {
                        draggedTabId: source.data.tabId,
                    });
                    return;
                }

                // Commit-on-release tear-off: the tab was released BELOW the
                // strip (dragged down into the window body) and let go.
                // Spawn the new window at the release point NOW — deliberately
                // not mid-drag, so nothing detaches until the user releases
                // (the behaviour they expect).
                if (release === "tear-off") {
                    // Clear the payload SYNCHRONOUSLY (before the async
                    // tear-off) so CrossWindowDragMonitor's dragend handler —
                    // which may fire for the same gesture — sees no payload
                    // and doesn't double-process this tear.
                    setCurrentDragPayload(null);
                    tearOffTabAtRelease(draggedTabId, input);
                    setInsertionPoint(null);
                    return;
                }

                const willReorder = release === "reorder";

                if (willReorder || location.current.dropTargets.length > 0) {
                    setCurrentDragPayload(null);
                }

                if (willReorder && ip != null) {
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
        //    every click — so it runs whenever the tile session ends:
        //    normally from pragmatic's monitor onDrop below, otherwise from
        //    the drag session's safety net (a window dragend pragmatic's
        //    dispatch skipped on Win11 swallowed-drag paths, or a pointerdown
        //    after a drag that ended unobserved) or a new drag replacing a
        //    stranded one.
        const cleanupTileDragState = () => {
            setHoveredDropTabId(null);
            clearCrossTabDrop();
            // Reset EVERY tab's overlay, not just the spring-activated
            // set: the SOURCE tab's activeDrag is normally reset by its
            // own draggable's onDrop, but that dispatch is skipped
            // whenever dragend is suppressed (swallowed-drag paths,
            // source unmounted early, …) — and a stuck overlay is a dead
            // tab. Safe here because this only runs at end-of-drag.
            for (const tabId of tabIds()) {
                getLayoutModelForTabById(tabId)?.activeDrag._set(false);
            }
            dragActivatedTabIds.clear();
            // Deferred dangling-leaf prune: mid-drag pruning is gated off
            // (see pruneDanglingLeaves), so the source tab's disowned
            // leaf is removed HERE, after the gesture and the move RPC's
            // Tab updates have settled. 250ms comfortably covers the
            // observed 20-40ms RPC round-trip.
            setTimeout(() => {
                for (const tabId of tabIds()) {
                    const model = getLayoutModelForTabById(tabId);
                    if (model) pruneDanglingLeaves(model);
                }
            }, 250);
        };
        const stopTileSubscriber = onSessionEnded(({ session: s, reason }) => {
            if (s.kind !== "tile") return;
            if (reason === "unobserved") {
                // The drag ended with no monitor onDrop and no window
                // dragend: log each tab's overlay state so field diags can
                // pinpoint what wedged. (SPEC_PANE_DRAG_TO_TAB addendum A2.)
                Logger.warn("dnd", "pointerdown net fired — drag ended unobserved", {
                    activated: [...dragActivatedTabIds],
                    overlays: tabIds().map((id) => ({
                        tabId: id,
                        activeDrag: getLayoutModelForTabById(id)?.activeDrag() ?? null,
                    })),
                });
            }
            cleanupTileDragState();
        });
        const cleanupTileMonitor = monitorForElements({
            canMonitor: ({ source }) => source.data.type === tileItemType,
            onDrop: () => {
                // Ending the session runs the cleanup (the subscriber above).
                if (session()?.kind === "tile") endTileDrag("drop");
                else cleanupTileDragState();
            },
        });

        onCleanup(() => {
            cleanupStripDrop();
            cleanupTileMonitor();
            stopTileSubscriber();
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
