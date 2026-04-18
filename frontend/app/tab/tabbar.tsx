// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, createTab, setActiveTab } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { monitorForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { For, onCleanup, onMount } from "solid-js";
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


function TabBar(props: TabBarProps): JSX.Element {
    const activeTabId = atoms.activeTabId;

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
        // #3b82f6 is the "Blue" entry in TAB_COLORS — same as the color
        // picker's blue swatch, so stays consistent with user-chosen blue.
        fireAndForget(async () => {
            try {
                await ObjectService.UpdateObjectMeta(
                    makeORef("tab", firstId),
                    { "tab:color": "#3b82f6" } as MetaType,
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
            onDrag: ({ location }) => {
                setInsertionPoint(computeInsertionPoint(location.current.input.clientX));
            },

            onDrop: ({ source, location }) => {
                // Only clear cross-window payload when there's a real in-window drop target.
                // monitorForElements.onDrop fires for ALL drags (including out-of-window),
                // so check dropTargets to distinguish a valid drop from a drag that ended
                // outside the window (where CrossWindowDragMonitor should handle it instead).
                if (location.current.dropTargets.length > 0) {
                    setCurrentDragPayload(null);
                }

                const ip = insertionPoint();
                const draggedTabId = source.data.tabId as string;

                if (ip && draggedTabId) {
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

    if (!props.workspace) return null;

    const activeIndex = () => tabIds().indexOf(activeTabId());

    return (
        <div class="tab-bar" {...dragProps}>
            <button class="add-tab-btn" onClick={createTab} title="New Tab" data-tauri-drag-region="false">
                <i class="fa fa-plus" />
            </button>
            <div class="tab-bar-scroll" data-tauri-drag-region="false">
                <For each={tabIds()}>
                    {(tabId, i) => (
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
                    )}
                </For>
            </div>
            {/* Empty right-side space — draggable so the user can grab the window from here */}
            <div class="tab-bar-fill" data-tauri-drag-region="true" />
        </div>
    );
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
