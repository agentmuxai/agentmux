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
import { TAB_COLORS } from "./tab";
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

    const pinnedTabIds = () => props.workspace?.pinnedtabids ?? [];
    const regularTabIds = () => props.workspace?.tabids ?? [];
    const allTabIds = () => [...pinnedTabIds(), ...regularTabIds()];

    const handleSelect = (tabId: string) => {
        if (tabId !== activeTabId()) setActiveTab(tabId);
    };

    const handleClose = (tabId: string) => {
        const allTabs = allTabIds();
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

    const handlePinChange = (tabId: string) => {
        const pinned = pinnedTabIds();
        const regular = regularTabIds();
        const isPinned = pinned.includes(tabId);
        const newPinnedIds = isPinned ? pinned.filter((id) => id !== tabId) : [...pinned, tabId];
        const newRegularIds = isPinned ? [...regular, tabId] : regular.filter((id) => id !== tabId);

        // Auto-assign a color when the first tab is pinned and the tab has no color yet
        const isFirstPin = !isPinned && pinned.length === 0;
        const tab = isFirstPin ? getObjectValue<Tab>(makeORef("tab", tabId)) : null;
        const needsColor = isFirstPin && !tab?.meta?.["tab:color"];

        fireAndForget(async () => {
            await WorkspaceService.UpdateTabIds(props.workspace.oid, newRegularIds, newPinnedIds);
            if (needsColor) {
                const allIds = [...(props.workspace.tabids ?? []), ...(props.workspace.pinnedtabids ?? [])];
                const usedColors = allIds.map((id) => {
                    const t = getObjectValue<Tab>(makeORef("tab", id));
                    return t?.meta?.["tab:color"] as string | null | undefined;
                });
                const palette = TAB_COLORS.map((c) => c.hex);
                const available = palette.filter((hex) => !usedColors.includes(hex));
                const pool = available.length > 0 ? available : palette;
                const color = pool[Math.floor(Math.random() * pool.length)];
                await ObjectService.UpdateObjectMeta(makeORef("tab", tabId), { "tab:color": color } as MetaType);
            }
        });
    };

    const { dragProps } = useWindowDrag();

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
                    const pinned = pinnedTabIds();
                    const regular = regularTabIds();
                    const wsId = props.workspace?.oid;

                    executeReorder(ip, draggedTabId, pinned, regular, wsId);

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

    const activeIndex = () => allTabIds().indexOf(activeTabId());

    return (
        <div class="tab-bar" {...dragProps}>
            <button class="add-tab-btn" onClick={createTab} title="New Tab" data-tauri-drag-region="false">
                <i class="fa fa-plus" />
            </button>
            <div class="tab-bar-scroll" data-tauri-drag-region="false">
                <For each={pinnedTabIds()}>
                    {(tabId, i) => (
                        <DroppableTab
                            tabId={tabId}
                            workspaceId={props.workspace.oid}
                            activeTabId={activeTabId()}
                            isActive={tabId === activeTabId()}
                            isFirst={i() === 0}
                            isBeforeActive={i() === activeIndex() - 1}
                            isPinned={true}
                            allTabCount={allTabIds().length}
                            tabIndex={i()}
                            sectionIndex={i()}
                            pinnedTabIds={pinnedTabIds()}
                            regularTabIds={regularTabIds()}
                            onSelect={() => handleSelect(tabId)}
                            onClose={() => handleClose(tabId)}
                            onPinChange={() => handlePinChange(tabId)}
                        />
                    )}
                </For>
                {pinnedTabIds().length > 0 && <div class="pinned-tab-spacer" />}
                <For each={regularTabIds()}>
                    {(tabId, i) => (
                        <DroppableTab
                            tabId={tabId}
                            workspaceId={props.workspace.oid}
                            activeTabId={activeTabId()}
                            isActive={tabId === activeTabId()}
                            isFirst={pinnedTabIds().length === 0 && i() === 0}
                            isBeforeActive={pinnedTabIds().length + i() === activeIndex() - 1}
                            isPinned={false}
                            allTabCount={allTabIds().length}
                            tabIndex={pinnedTabIds().length + i()}
                            sectionIndex={i()}
                            pinnedTabIds={pinnedTabIds()}
                            regularTabIds={regularTabIds()}
                            onSelect={() => handleSelect(tabId)}
                            onClose={() => handleClose(tabId)}
                            onPinChange={() => handlePinChange(tabId)}
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
 * Execute the reorder or cross-section move described by the insertion point.
 * All drop logic lives here — droppable-tab.tsx is visual-only.
 */
function executeReorder(
    ip: InsertionPoint,
    draggedTabId: string,
    pinned: string[],
    regular: string[],
    wsId: string
): void {
    const sourceInPinned = pinned.includes(draggedTabId);

    // Classify which section the gap belongs to
    const beforeInPinned = ip.beforeTabId ? pinned.includes(ip.beforeTabId) : null;
    const afterInPinned  = ip.afterTabId  ? pinned.includes(ip.afterTabId)  : null;

    let targetSection: "pinned" | "regular";
    let insertIdx: number;

    if (ip.beforeTabId === null) {
        // Gap before the very first tab
        targetSection = afterInPinned ? "pinned" : "regular";
        insertIdx = 0;
    } else if (ip.afterTabId === null) {
        // Gap after the very last tab
        targetSection = beforeInPinned ? "pinned" : "regular";
        const section = targetSection === "pinned" ? pinned : regular;
        insertIdx = section.length;
    } else if (beforeInPinned === afterInPinned) {
        // Gap within same section — insert before afterTabId
        targetSection = beforeInPinned ? "pinned" : "regular";
        const section = targetSection === "pinned" ? pinned : regular;
        insertIdx = section.indexOf(ip.afterTabId!);
    } else {
        // Cross-section boundary gap: beforeTabId is pinned, afterTabId is regular (or vice versa)
        // Snap to the section of whichever tab is closest to the gap's logical side
        if (sourceInPinned) {
            // Pinned tab dropped at boundary → moves to start of regular
            targetSection = "regular";
            insertIdx = 0;
        } else {
            // Regular tab dropped at boundary → moves to end of pinned
            targetSection = "pinned";
            insertIdx = pinned.length;
        }
    }

    if (sourceInPinned === (targetSection === "pinned")) {
        // Same-section reorder — use ReorderTab (remove-then-insert)
        const section = targetSection === "pinned" ? pinned : regular;
        const sourceIdx = section.indexOf(draggedTabId);
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
    } else {
        // Cross-section move — use UpdateTabIds
        const newPinned = [...pinned];
        const newRegular = [...regular];
        if (sourceInPinned) {
            newPinned.splice(newPinned.indexOf(draggedTabId), 1);
            newRegular.splice(Math.min(insertIdx, newRegular.length), 0, draggedTabId);
        } else {
            newRegular.splice(newRegular.indexOf(draggedTabId), 1);
            newPinned.splice(Math.min(insertIdx, newPinned.length), 0, draggedTabId);
        }
        Logger.info("dnd", "tab-cross-section drop", { draggedTabId, targetSection, insertIdx });
        fireAndForget(() => WorkspaceService.UpdateTabIds(wsId, newRegular, newPinned));
    }
}

export { TabBar };
