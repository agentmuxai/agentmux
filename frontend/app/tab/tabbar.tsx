// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, createTab, setActiveTab } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { monitorForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { For, onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import { WorkspaceService } from "../store/services";
import { deleteLayoutModelForTab } from "@/layout/index";
import { DroppableTab } from "./droppable-tab";
import {
    tabItemType,
    globalDragTabId,
    nearestHint,
    setNearestHint,
    computeNearestTab,
    computeInsertIndex,
} from "./tabbar-dnd";
import { Logger } from "@/util/logger";
import "./tabbar.scss";

export { tabItemType } from "./tabbar-dnd";

interface TabBarProps {
    workspace: Workspace;
}

function NewTabDropZone(): JSX.Element {
    return (
        <div
            class="new-tab-drop-zone"
            title="Drop here to create new tab"
            data-tauri-drag-region="true"
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
        fireAndForget(() => WorkspaceService.UpdateTabIds(props.workspace.oid, newRegularIds, newPinnedIds));
    };

    const { dragProps } = useWindowDrag();

    // Global drag monitor: tracks cursor when it's outside all tab drop targets.
    // When dropped outside, executes the reorder to the nearest tab.
    onMount(() => {
        const cleanup = monitorForElements({
            canMonitor: ({ source }) => source.data.type === tabItemType,
            onDrag: ({ location }) => {
                if (location.current.dropTargets.length > 0) {
                    setNearestHint(null);
                    return;
                }
                setNearestHint(computeNearestTab(
                    location.current.input.clientX,
                    location.current.input.clientY,
                ));
            },
            onDrop: () => {
                const hint = nearestHint();
                if (hint && globalDragTabId && globalDragTabId !== hint.tabId) {
                    const pinned = pinnedTabIds();
                    const regular = regularTabIds();
                    const sourceInPinned = pinned.includes(globalDragTabId);
                    const targetInPinned = pinned.includes(hint.tabId);

                    if (sourceInPinned === targetInPinned) {
                        const section = sourceInPinned ? pinned : regular;
                        const sourceIdx = section.indexOf(globalDragTabId);
                        const targetIdx = section.indexOf(hint.tabId);

                        if (sourceIdx >= 0 && targetIdx >= 0) {
                            const newIndex = computeInsertIndex(sourceIdx, targetIdx, hint.side);
                            const draggedTabId = globalDragTabId;
                            const wsId = props.workspace?.oid;
                            Logger.info("dnd", "tab-reorder drop (nearest)", {
                                draggedTabId,
                                targetTabId: hint.tabId,
                                side: hint.side,
                                sourceIdx,
                                targetIdx,
                                newIndex,
                                workspaceId: wsId,
                            });
                            fireAndForget(async () => {
                                try {
                                    await WorkspaceService.ReorderTab(wsId, draggedTabId, newIndex);
                                } catch (e) {
                                    Logger.error("dnd", "tab-reorder failed", {
                                        tabId: draggedTabId,
                                        newIndex,
                                        error: String(e),
                                    });
                                }
                            });
                        }
                    }
                }
                setNearestHint(null);
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
            <div class="tab-bar-scroll">
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
                            onSelect={() => handleSelect(tabId)}
                            onClose={() => handleClose(tabId)}
                            onPinChange={() => handlePinChange(tabId)}
                        />
                    )}
                </For>
                <NewTabDropZone />
            </div>
        </div>
    );
}

export { TabBar };
