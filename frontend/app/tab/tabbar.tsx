// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, createTab, getApi, setActiveTab } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { monitorForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { For, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { ObjectService, WorkspaceService } from "../store/services";
import { makeORef, getObjectValue } from "../store/wos";
import { deleteLayoutModelForTab } from "@/layout/index";
import { DroppableTab } from "./droppable-tab";
import {
    tabItemType,
    insertionPoint,
    setInsertionPoint,
    bouncingTabId,
    setBouncingTabId,
    computeInsertionPoint,
    InsertionPoint,
} from "./tabbar-dnd";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { Logger } from "@/util/logger";
import "./tabbar.scss";

export { tabItemType } from "./tabbar-dnd";

interface TabBarProps {
    workspace: Workspace;
}


// Pixels past the tab strip's bottom edge before a drag becomes a
// tear-off (Chrome uses a similar small threshold). 24 px is enough
// to filter out brief excursions while the user is still hunting for
// the drop position; small enough that the tear feels intentional.
// See docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26 §4.1.
const TEAR_PAST_PX = 24;

function TabBar(props: TabBarProps): JSX.Element {
    const activeTabId = atoms.activeTabId;
    let tabBarScrollRef!: HTMLDivElement;
    // Latches once per drag — the tear-off handshake should only run a
    // single time even if the user keeps moving past the threshold.
    let tearOffFired = false;

    // Pin feature removed — merge any legacy pinnedtabids into the regular list
    // so existing workspaces don't lose tabs. A one-time UpdateTabIds (below)
    // drains pinnedtabids server-side so this concat becomes a no-op.
    const tabIds = () => {
        const ws = props.workspace;
        if (!ws) return [];
        return [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
    };

    const handleSelect = (tabId: string) => {
        if (tabId !== activeTabId()) setActiveTab(tabId);
    };

    const handleClose = (tabId: string) => {
        const allTabs = tabIds();
        if (allTabs.length <= 1) return;
        fireAndForget(async () => {
            if (tabId === activeTabId()) {
                const idx = allTabs.indexOf(tabId);
                const nextTab = allTabs[idx + 1] ?? allTabs[idx - 1];
                if (nextTab) await setActiveTab(nextTab);
            }
            await WorkspaceService.CloseTab(props.workspace.oid, tabId);
            deleteLayoutModelForTab(tabId);
        });
    };

    const { dragProps } = useWindowDrag();

    // One-time migration: if this workspace still has pinned tabs from an older
    // build, fold them into tabids and clear pinnedtabids server-side.
    onMount(() => {
        const ws = props.workspace;
        if (ws && (ws.pinnedtabids?.length ?? 0) > 0) {
            const merged = [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
            fireAndForget(async () => {
                try {
                    await WorkspaceService.UpdateTabIds(ws.oid, merged, []);
                } catch (e) {
                    console.error("[tabbar] pin migration failed:", e);
                }
            });
        }
    });

    // Startup-tab color: if the workspace has exactly one tab and it has no
    // tab:color set, apply the theme Blue from TAB_COLORS. The backend-created
    // startup tab doesn't get a color meta, so without this the first tab
    // stays neutral while every user-created tab is vibrant.
    onMount(() => {
        const ws = props.workspace;
        if (!ws) return;
        const ids = [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
        if (ids.length !== 1) return;
        const firstId = ids[0];
        const tab = getObjectValue<Tab>(makeORef("tab", firstId));
        if (!tab) return;
        if (tab.meta?.["tab:color"]) return;
        // Skip if the default-color backfill has already run once for this
        // tab. Without this guard, a user who clears the color on a single-
        // tab workspace would have it silently restored on the next mount.
        if (tab.meta?.["tab:color-initialized"]) return;
        // #3b82f6 is the "Blue" entry in TAB_COLORS — same as the color
        // picker's blue swatch, so stays consistent with user-chosen blue.
        fireAndForget(async () => {
            try {
                await ObjectService.UpdateObjectMeta(
                    makeORef("tab", firstId),
                    { "tab:color": "#3b82f6", "tab:color-initialized": true } as MetaType,
                );
            } catch (e) {
                console.error("[tabbar] startup-tab color apply failed:", e);
            }
        });
    });

    onMount(() => {
        const cleanup = monitorForElements({
            canMonitor: ({ source }) => source.data.type === tabItemType,

            // Always compute insertion point from cursor position — drives the gap animation on all tabs
            onDrag: ({ source, location }) => {
                const input = location.current.input;
                const rect = tabBarScrollRef?.getBoundingClientRect();
                if (rect && !tearOffFired && input.clientY > rect.bottom + TEAR_PAST_PX) {
                    tearOffFired = true;
                    const draggedTabId = source.data.tabId as string;
                    const wsId = props.workspace?.oid;
                    if (wsId) {
                        // openWindowAtPosition + SC_MOVE expect SCREEN
                        // coordinates, but `input.clientX/Y` are
                        // viewport-relative. Convert via window.screenX/Y
                        // (works across multi-monitor setups).
                        const screenX = window.screenX + input.clientX;
                        const screenY = window.screenY + input.clientY;
                        // Phase 2: real tear-off (sidecar TearOffTab +
                        // host openWindowAtPosition + Win32 SC_MOVE).
                        // Fire-and-forget — the user is mid-drag and the
                        // host's SC_MOVE handshake will take over the
                        // cursor synchronously from Windows' point of view.
                        //
                        // The cross-window drag payload is cleared INSIDE
                        // requestTearOff after the SC_MOVE handshake
                        // returns successfully — clearing it here would
                        // strand the legacy dragend fallback if any step
                        // (TearOffTab, openWindowAtPosition, handshake)
                        // failed mid-flight, leaving the gesture as a
                        // silent no-op.
                        fireAndForget(() =>
                            requestTearOff(draggedTabId, wsId, screenX, screenY),
                        );
                    }
                    // Continue updating insertionPoint below — until the
                    // host's SC_MOVE handshake completes (~150-300ms cold
                    // path), the user's mouse is still over our window
                    // and pragmatic-dnd is still firing onDrag. The
                    // tearOffFired latch ensures requestTearOff doesn't
                    // re-fire; the gap-tracking is harmless during the
                    // brief overlap.
                }
                setInsertionPoint(computeInsertionPoint(input.clientX));
            },

            onDrop: ({ source, location }) => {
                // Reset the tear-off latch for the next drag.
                // NOTE: even though Phase 2's requestTearOff now does a
                // real handshake and the SC_MOVE takes over the cursor,
                // pragmatic-dnd's onDrop still fires for the original
                // gesture. We don't short-circuit here because a drag
                // that briefly dipped past the strip's bottom and then
                // came back to drop on a tab should still reorder
                // (Chrome treats the threshold as commit-on-cross, but
                // requestTearOff already snapshotted via a fire-and-
                // forget, so the data path is settled by the time onDrop
                // runs). Once Phase 5 (cancel-back) ships, this gate
                // tightens further.
                tearOffFired = false;

                const ip = insertionPoint();
                const draggedTabId = source.data.tabId as string;

                // `insertionPoint` reflects the last cursor X, so it can be
                // non-null even when the user has dragged BELOW the tab bar
                // for a tear-off. Gate both the payload-clear and the
                // reorder on the cursor actually being over the tab strip
                // at drop time — otherwise we'd suppress tear-offs and
                // also reorder on out-of-bar drops. Pragmatic-dnd does not
                // register a drop target for the bar (insertion is purely
                // X-driven), so we hit-test the cursor against the strip's
                // bounding rect ourselves.
                const input = location.current.input;
                const rect = tabBarScrollRef?.getBoundingClientRect();
                const dropInsideBar =
                    rect != null &&
                    input.clientY >= rect.top && input.clientY <= rect.bottom &&
                    input.clientX >= rect.left && input.clientX <= rect.right;
                const willReorder = dropInsideBar && ip != null && draggedTabId != null;

                if (willReorder || location.current.dropTargets.length > 0) {
                    setCurrentDragPayload(null);
                }

                if (willReorder) {
                    const tabs = tabIds();
                    const wsId = props.workspace?.oid;

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
        if (!tabBarScrollRef) return;
        e.preventDefault();
        tabBarScrollRef.scrollLeft += delta;
    };

    onMount(() => {
        if (!tabBarScrollRef) return;
        // `passive: false` is required so preventDefault works.
        tabBarScrollRef.addEventListener("wheel", handleStripWheel, { passive: false });
        onCleanup(() => tabBarScrollRef?.removeEventListener("wheel", handleStripWheel));
    });

    if (!props.workspace) return null;

    const activeIndex = () => tabIds().indexOf(activeTabId());

    return (
        <div class="tab-bar" {...dragProps}>
            <button class="add-tab-btn" onClick={createTab} title="New Tab" data-drag-region="false">
                <i class="fa fa-plus" />
            </button>
            <div ref={tabBarScrollRef!} class="tab-bar-scroll" data-drag-region="false">
                <For each={tabIds()}>
                    {(tabId, i) => (
                        <>
                            {/* Real DOM separator between adjacent tabs (skipped
                                before index 0). Constant width + identical CSS
                                in every position guarantees uniform inter-tab
                                spacing, regardless of which tab is active /
                                hovered / dragged. Per
                                SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25 §3.4. */}
                            <Show when={i() > 0}>
                                <div class="tab-separator" aria-hidden="true" />
                            </Show>
                            <DroppableTab
                                tabId={tabId}
                                workspaceId={props.workspace.oid}
                                activeTabId={activeTabId()}
                                isActive={tabId === activeTabId()}
                                isFirst={i() === 0}
                                isBeforeActive={i() === activeIndex() - 1}
                                allTabCount={tabIds().length}
                                tabIndex={i()}
                                tabIds={tabIds()}
                                onSelect={() => handleSelect(tabId)}
                                onClose={() => handleClose(tabId)}
                            />
                        </>
                    )}
                </For>
            </div>
            {/* Empty right-side space — draggable so the user can grab the window from here */}
            <div class="tab-bar-fill" data-drag-region="true" />
        </div>
    );
}

/**
 * Phase 2 — orchestrates the Chrome-faithful tear-off when the cursor
 * crosses the strip's bottom edge. Three steps, all from the source
 * window's renderer:
 *
 *   1. Move the tab data to a brand-new workspace (sidecar).
 *   2. Spawn a new agentmux-cef window pointed at that workspace
 *      (host).
 *   3. Hand cursor capture over to the new window via Win32 SC_MOVE
 *      (host) so it follows the mouse like a Chrome torn-off tab.
 *
 * Phase 2 acceptance is structural: the SC_MOVE plumbing fires and
 * the window follows the cursor. The cold-path first-paint flash
 * (~150-300ms while the new window registers + paints) is expected
 * and is not an acceptance failure — Phase 6's pre-warmed pool
 * brings it to 0 ms. The ≤ 8 ms handshake budget from the spec §0
 * is measured by the host and emitted as `handshakeMs` on this
 * call's result.
 *
 * Subsequent phases:
 *   - Phase 3: capture a TabSnapshot for width preservation
 *   - Phase 4: WH_MOUSE_LL hook for cross-window merge detection
 *   - Phase 5: cancel-back-to-source on drop over origin strip
 *   - Phase 6: pre-warmed window pool (eliminates first-paint flash)
 *
 * See docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.
 */
async function requestTearOff(
    tabId: string,
    workspaceId: string,
    cursorX: number,
    cursorY: number,
): Promise<void> {
    const t0 = performance.now();
    try {
        const sourceWindowLabel = await getApi().getWindowLabel();
        // Step 1 — sidecar transfers the tab into a new workspace.
        // Returns the new workspace's ID.
        const newWsId = await WorkspaceService.TearOffTab(tabId, workspaceId);
        // Step 2 — host spawns the destination window pointed at the
        // new workspace. Returns the new window's label.
        const destWindowLabel = await getApi().openWindowAtPosition(
            cursorX,
            cursorY,
            newWsId,
        );
        // Step 3 — Win32 SC_MOVE handshake. Host waits for the new
        // window's HWND to register, then transfers cursor capture
        // and posts WM_SYSCOMMAND/SC_MOVE so Windows enters its
        // built-in modal move-loop. Until mouseup, the new window
        // follows the cursor at full opacity, no ghost.
        const result = await getApi().tearOffSCMoveHandshake({
            sourceWindowLabel,
            destWindowLabel,
            cursorX,
            cursorY,
        });
        // Handshake succeeded — Windows now owns the move loop. Clear
        // the cross-window drag payload so the legacy dragend pipeline
        // (CrossWindowDragMonitor) doesn't double-process this gesture
        // when its dragend fires. Cleared HERE rather than in onDrag so
        // a failure mid-pipeline (TearOffTab, openWindowAtPosition, or
        // the handshake itself) leaves the legacy fallback intact.
        setCurrentDragPayload(null);
        Logger.info("dnd", "tab tear-off complete", {
            tabId,
            destWindowLabel,
            handshakeMs: result.handshakeMs,
            totalMs: performance.now() - t0,
        });
    } catch (e) {
        Logger.error("dnd", "tab tear-off failed", { tabId, error: String(e) });
    }
}

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

export { TabBar };
