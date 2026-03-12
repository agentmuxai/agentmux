// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, createTab, setActiveTab } from "@/store/global";
import { Logger } from "@/util/logger";
import { fireAndForget } from "@/util/util";
import { For } from "solid-js";
import type { JSX } from "solid-js";
import { WorkspaceService } from "../store/services";
import { deleteLayoutModelForTab } from "@/layout/index";
import { Tab } from "./tab";
import "./tabbar.scss";

export const tabItemType = "TAB_ITEM";

interface TabBarProps {
    workspace: Workspace;
}

/**
 * Wraps a Tab component with plain DOM drag/drop support for tab reordering
 * and pane-to-tab drops. react-dnd has been removed.
 */
function DroppableTab(props: {
    tabId: string;
    workspaceId: string;
    activeTabId: string;
    isActive: boolean;
    isFirst: boolean;
    isBeforeActive: boolean;
    isPinned: boolean;
    allTabCount: number;
    tabIndex: number;
    onSelect: () => void;
    onClose: () => void;
    onPinChange: () => void;
}): JSX.Element {
    let tabWrapRef!: HTMLDivElement;

    const handleDragStart = (e: MouseEvent) => {
        if (props.allTabCount <= 1) return;
        // Use HTML5 drag via dataTransfer when available
        const dragEvent = e as unknown as DragEvent;
        if (dragEvent.dataTransfer) {
            dragEvent.dataTransfer.effectAllowed = "move";
            dragEvent.dataTransfer.setData(
                "application/x-tab-reorder",
                JSON.stringify({ tabId: props.tabId, workspaceId: props.workspaceId, isPinned: props.isPinned })
            );
        }
        Logger.info("dnd", "tab-drag started", {
            tabId: props.tabId,
            workspaceId: props.workspaceId,
            isPinned: props.isPinned,
        });
    };

    const handleDragOver = (e: DragEvent) => {
        e.preventDefault();
        if (e.dataTransfer) {
            e.dataTransfer.dropEffect = "move";
        }
    };

    const handleDrop = (e: DragEvent) => {
        e.preventDefault();
        const raw = e.dataTransfer?.getData("application/x-tab-reorder");
        if (!raw) return;
        try {
            const { tabId: draggedTabId } = JSON.parse(raw);
            if (draggedTabId === props.tabId) return;
            const rect = tabWrapRef.getBoundingClientRect();
            const midX = (rect.right - rect.left) / 2;
            const clientX = e.clientX - rect.left;
            const side = clientX < midX ? "left" : "right";
            const newIndex = side === "left" ? props.tabIndex : props.tabIndex + 1;
            Logger.info("dnd", "tab-reorder drop", {
                draggedTabId,
                targetTabId: props.tabId,
                side,
                newIndex,
                workspaceId: props.workspaceId,
            });
            fireAndForget(async () => {
                try {
                    await WorkspaceService.ReorderTab(props.workspaceId, draggedTabId, newIndex);
                } catch (e) {
                    Logger.error("dnd", "tab-reorder failed", {
                        tabId: draggedTabId,
                        newIndex,
                        error: String(e),
                    });
                }
            });
        } catch {
            // ignore parse errors
        }
    };

    return (
        <div
            ref={tabWrapRef!}
            class="tab-drop-wrapper"
            onDragOver={handleDragOver}
            onDrop={handleDrop}
        >
            <Tab
                id={props.tabId}
                active={props.isActive}
                isFirst={props.isFirst}
                isBeforeActive={props.isBeforeActive}
                isDragging={false}
                tabWidth={0}
                isNew={false}
                isPinned={props.isPinned}
                onSelect={props.onSelect}
                onClose={props.onClose}
                onDragStart={handleDragStart}
                onLoaded={() => {}}
                onPinChange={props.onPinChange}
            />
        </div>
    );
}

/**
 * Drop zone at the end of the tab bar that creates a new tab from a dropped pane.
 */
function NewTabDropZone(props: { workspaceId: string }): JSX.Element {
    const handleDragOver = (e: DragEvent) => {
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    };

    return (
        <div
            class="new-tab-drop-zone"
            onDragOver={handleDragOver}
            title="Drop here to create new tab"
        >
            <i class="fa fa-plus" />
        </div>
    );
}

function TabBar(props: TabBarProps): JSX.Element {
    const activeTabId = atoms.activeTabId;

    const pinnedTabIds = () => props.workspace?.pinnedtabids ?? [];
    const regularTabIds = () => props.workspace?.tabids ?? [];
    const allTabIds = () => [...pinnedTabIds(), ...regularTabIds()];

    const handleSelect = (tabId: string) => {
        if (tabId !== activeTabId()) {
            setActiveTab(tabId);
        }
    };

    const handleClose = (tabId: string) => {
        const allTabs = allTabIds();
        if (allTabs.length <= 1) return;

        fireAndForget(async () => {
            if (tabId === activeTabId()) {
                const idx = allTabs.indexOf(tabId);
                const nextTab = allTabs[idx + 1] ?? allTabs[idx - 1];
                if (nextTab) {
                    await setActiveTab(nextTab);
                }
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
        fireAndForget(() => WorkspaceService.UpdateTabIds(props.workspace.oid, newRegularIds, newPinnedIds));
    };

    const handleAddTab = () => {
        createTab();
    };

    if (!props.workspace) return null;

    const activeIndex = () => allTabIds().indexOf(activeTabId());

    return (
        <div class="tab-bar" data-tauri-drag-region="false">
            <button class="add-tab-btn" onClick={handleAddTab} title="New Tab">
                <i class="fa fa-plus" />
            </button>
            <div class="tab-bar-scroll">
                <For each={pinnedTabIds()}>
                    {(tabId, i) => {
                        const idx = i();
                        const isActive = () => tabId === activeTabId();
                        const isBeforeActive = () => idx === activeIndex() - 1;
                        return (
                            <DroppableTab
                                tabId={tabId}
                                workspaceId={props.workspace.oid}
                                activeTabId={activeTabId()}
                                isActive={isActive()}
                                isFirst={i() === 0}
                                isBeforeActive={isBeforeActive()}
                                isPinned={true}
                                allTabCount={allTabIds().length}
                                tabIndex={idx}
                                onSelect={() => handleSelect(tabId)}
                                onClose={() => handleClose(tabId)}
                                onPinChange={() => handlePinChange(tabId)}
                            />
                        );
                    }}
                </For>
                {pinnedTabIds().length > 0 && <div class="pinned-tab-spacer" />}
                <For each={regularTabIds()}>
                    {(tabId, i) => {
                        const idx = () => pinnedTabIds().length + i();
                        const isActive = () => tabId === activeTabId();
                        const isBeforeActive = () => idx() === activeIndex() - 1;
                        return (
                            <DroppableTab
                                tabId={tabId}
                                workspaceId={props.workspace.oid}
                                activeTabId={activeTabId()}
                                isActive={isActive()}
                                isFirst={pinnedTabIds().length === 0 && i() === 0}
                                isBeforeActive={isBeforeActive()}
                                isPinned={false}
                                allTabCount={allTabIds().length}
                                tabIndex={idx()}
                                onSelect={() => handleSelect(tabId)}
                                onClose={() => handleClose(tabId)}
                                onPinChange={() => handlePinChange(tabId)}
                            />
                        );
                    }}
                </For>
                <NewTabDropZone workspaceId={props.workspace.oid} />
            </div>
        </div>
    );
}

export { TabBar };
