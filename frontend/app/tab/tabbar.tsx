// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, createTab, globalStore, setActiveTab } from "@/store/global";
import { Logger } from "@/util/logger";
import { fireAndForget } from "@/util/util";
import { useAtomValue } from "jotai";
import { memo, useCallback, useRef } from "react";
import { useDrag, useDrop } from "react-dnd";
import { WorkspaceService } from "../store/services";
import { deleteLayoutModelForTab } from "@/layout/index";
import { tileItemType } from "@/layout/index";
import type { LayoutNode } from "@/layout/index";
import { Tab } from "./tab";
import "./tabbar.scss";

export const tabItemType = "TAB_ITEM";

interface TabBarProps {
    workspace: Workspace;
}

/**
 * Wraps a Tab component to make it a drop target for TILE_ITEM (panes)
 * and a drag source / drop target for TAB_ITEM (tab reordering).
 */
const DroppableTab = memo(
    ({
        tabId,
        workspaceId,
        activeTabId,
        isActive,
        isFirst,
        isBeforeActive,
        isPinned,
        allTabCount,
        tabIndex,
        onSelect,
        onClose,
        onPinChange,
    }: {
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
    }) => {
        const tabRef = useRef<HTMLDivElement>(null);

        // --- TILE_ITEM drop target (pane → tab) ---
        const [{ isOverTile, canDropTile }, tileDropRef] = useDrop(
            () => ({
                accept: tileItemType,
                canDrop: () => true,
                drop: (item: LayoutNode) => {
                    const sourceTabId = globalStore.get(atoms.activeTabId);
                    const blockId = item.data?.blockId;
                    if (!blockId || sourceTabId === tabId) {
                        Logger.debug("dnd", "tile-drop on tab ignored (same tab or no blockId)", { blockId, sourceTabId, tabId });
                        return;
                    }
                    Logger.info("dnd", "tile-drop on tab: moving pane to tab", { blockId, sourceTabId, destTabId: tabId, workspaceId });
                    fireAndForget(async () => {
                        try {
                            await WorkspaceService.MoveBlockToTab(workspaceId, blockId, sourceTabId, tabId, true);
                            Logger.info("dnd", "tile-drop on tab: move complete", { blockId, destTabId: tabId });
                        } catch (e) {
                            Logger.error("dnd", "tile-drop on tab: MoveBlockToTab failed", { blockId, sourceTabId, destTabId: tabId, error: String(e) });
                        }
                    });
                },
                collect: (monitor) => ({
                    isOverTile: monitor.isOver({ shallow: true }),
                    canDropTile: monitor.canDrop(),
                }),
            }),
            [tabId, workspaceId]
        );

        // --- TAB_ITEM drag source (tab reordering) ---
        const [{ isDraggingTab }, tabDragRef] = useDrag(
            () => ({
                type: tabItemType,
                item: () => {
                    Logger.info("dnd", "tab-drag started", { tabId, workspaceId, isPinned, allTabCount });
                    return { tabId, workspaceId, isPinned };
                },
                canDrag: () => allTabCount > 1,
                end: (_item, monitor) => {
                    const didDrop = monitor.didDrop();
                    Logger.info("dnd", "tab-drag ended", { tabId, didDrop });
                },
                collect: (monitor) => ({
                    isDraggingTab: monitor.isDragging(),
                }),
            }),
            [tabId, workspaceId, isPinned, allTabCount]
        );

        // --- TAB_ITEM drop target (tab reordering) ---
        const [{ isOverTab, insertSide }, tabDropRef] = useDrop(
            () => ({
                accept: tabItemType,
                canDrop: (item: { tabId: string }) => item.tabId !== tabId,
                hover: (item: { tabId: string }, monitor) => {
                    if (!tabRef.current || item.tabId === tabId) return;
                    const hoverRect = tabRef.current.getBoundingClientRect();
                    const clientOffset = monitor.getClientOffset();
                    if (!clientOffset) return;
                    const hoverMiddleX = (hoverRect.right - hoverRect.left) / 2;
                    const hoverClientX = clientOffset.x - hoverRect.left;
                    // Store insert side for visual indicator
                    (tabRef.current as any).__insertSide = hoverClientX < hoverMiddleX ? "left" : "right";
                },
                drop: (item: { tabId: string; isPinned: boolean }) => {
                    if (item.tabId === tabId) return;
                    const side = (tabRef.current as any)?.__insertSide ?? "right";
                    const newIndex = side === "left" ? tabIndex : tabIndex + 1;
                    Logger.info("dnd", "tab-reorder drop", { draggedTabId: item.tabId, targetTabId: tabId, side, newIndex, workspaceId });
                    fireAndForget(async () => {
                        try {
                            await WorkspaceService.ReorderTab(workspaceId, item.tabId, newIndex);
                            Logger.info("dnd", "tab-reorder complete", { tabId: item.tabId, newIndex });
                        } catch (e) {
                            Logger.error("dnd", "tab-reorder failed", { tabId: item.tabId, newIndex, error: String(e) });
                        }
                    });
                },
                collect: (monitor) => ({
                    isOverTab: monitor.isOver({ shallow: true }) && monitor.canDrop(),
                    insertSide: (tabRef.current as any)?.__insertSide ?? "right",
                }),
            }),
            [tabId, workspaceId, tabIndex]
        );

        // Combine refs: tile drop + tab drag + tab drop
        const combinedRef = useCallback(
            (node: HTMLDivElement | null) => {
                (tabRef as any).current = node;
                tileDropRef(node);
                tabDragRef(node);
                tabDropRef(node);
            },
            [tileDropRef, tabDragRef, tabDropRef]
        );

        const noop = useCallback(() => {}, []);

        const tileHighlight = isOverTile && canDropTile;

        return (
            <div
                ref={combinedRef}
                className={
                    "tab-drop-wrapper" +
                    (tileHighlight ? " tile-drop-hover" : "") +
                    (isDraggingTab ? " tab-dragging" : "") +
                    (isOverTab ? ` tab-insert-${insertSide}` : "")
                }
            >
                <Tab
                    ref={null}
                    id={tabId}
                    active={isActive}
                    isFirst={isFirst}
                    isBeforeActive={isBeforeActive}
                    isDragging={isDraggingTab}
                    tabWidth={0}
                    isNew={false}
                    isPinned={isPinned}
                    onSelect={onSelect}
                    onClose={onClose}
                    onDragStart={noop}
                    onLoaded={noop}
                    onPinChange={onPinChange}
                />
            </div>
        );
    }
);
DroppableTab.displayName = "DroppableTab";

/**
 * Drop zone at the end of the tab bar that creates a new tab from a dropped pane.
 */
const NewTabDropZone = memo(({ workspaceId }: { workspaceId: string }) => {
    const [{ isOver, canDrop }, dropRef] = useDrop(
        () => ({
            accept: tileItemType,
            canDrop: () => true,
            drop: (item: LayoutNode) => {
                const sourceTabId = globalStore.get(atoms.activeTabId);
                const blockId = item.data?.blockId;
                if (!blockId) {
                    Logger.debug("dnd", "new-tab-zone drop ignored (no blockId)");
                    return;
                }
                Logger.info("dnd", "new-tab-zone drop: promoting pane to new tab", { blockId, sourceTabId, workspaceId });
                fireAndForget(async () => {
                    try {
                        await WorkspaceService.PromoteBlockToTab(workspaceId, blockId, sourceTabId, true);
                        Logger.info("dnd", "new-tab-zone drop: promote complete", { blockId });
                    } catch (e) {
                        Logger.error("dnd", "new-tab-zone drop: PromoteBlockToTab failed", { blockId, sourceTabId, error: String(e) });
                    }
                });
            },
            collect: (monitor) => ({
                isOver: monitor.isOver({ shallow: true }),
                canDrop: monitor.canDrop(),
            }),
        }),
        [workspaceId]
    );

    const highlight = isOver && canDrop;

    return (
        <div ref={dropRef} className={"new-tab-drop-zone" + (highlight ? " drop-hover" : "")} title="Drop here to create new tab">
            <i className="fa fa-plus" />
        </div>
    );
});
NewTabDropZone.displayName = "NewTabDropZone";

const TabBar = memo(({ workspace }: TabBarProps) => {
    const activeTabId = useAtomValue(atoms.activeTabId);

    const pinnedTabIds = workspace?.pinnedtabids ?? [];
    const regularTabIds = workspace?.tabids ?? [];
    const allTabIds = [...pinnedTabIds, ...regularTabIds];

    const handleSelect = useCallback(
        (tabId: string) => {
            if (tabId !== activeTabId) {
                setActiveTab(tabId);
            }
        },
        [activeTabId]
    );

    const handleClose = useCallback(
        (tabId: string) => {
            const allTabs = [...pinnedTabIds, ...regularTabIds];
            if (allTabs.length <= 1) return; // never close last tab

            fireAndForget(async () => {
                // If closing the active tab, switch to an adjacent tab first
                // and await the backend round-trip to prevent race conditions
                if (tabId === activeTabId) {
                    const idx = allTabs.indexOf(tabId);
                    const nextTab = allTabs[idx + 1] ?? allTabs[idx - 1];
                    if (nextTab) {
                        await setActiveTab(nextTab);
                    }
                }
                await WorkspaceService.CloseTab(workspace.oid, tabId);
                deleteLayoutModelForTab(tabId);
            });
        },
        [workspace?.oid, pinnedTabIds, regularTabIds, activeTabId]
    );

    const handlePinChange = useCallback(
        (tabId: string) => {
            const isPinned = pinnedTabIds.includes(tabId);
            const newPinnedIds = isPinned ? pinnedTabIds.filter((id) => id !== tabId) : [...pinnedTabIds, tabId];
            const newRegularIds = isPinned ? [...regularTabIds, tabId] : regularTabIds.filter((id) => id !== tabId);
            fireAndForget(() => WorkspaceService.UpdateTabIds(workspace.oid, newRegularIds, newPinnedIds));
        },
        [workspace?.oid, pinnedTabIds, regularTabIds]
    );

    const handleAddTab = useCallback(() => {
        createTab();
    }, []);

    if (!workspace) return null;

    const activeIndex = allTabIds.indexOf(activeTabId);

    return (
        <div className="tab-bar" data-tauri-drag-region="false">
            <button className="add-tab-btn" onClick={handleAddTab} title="New Tab">
                <i className="fa fa-plus" />
            </button>
            <div className="tab-bar-scroll">
                {pinnedTabIds.map((tabId, i) => {
                    const idx = i;
                    const isActive = tabId === activeTabId;
                    const isBeforeActive = idx === activeIndex - 1;
                    return (
                        <DroppableTab
                            key={tabId}
                            tabId={tabId}
                            workspaceId={workspace.oid}
                            activeTabId={activeTabId}
                            isActive={isActive}
                            isFirst={i === 0}
                            isBeforeActive={isBeforeActive}
                            isPinned={true}
                            allTabCount={allTabIds.length}
                            tabIndex={idx}
                            onSelect={() => handleSelect(tabId)}
                            onClose={() => handleClose(tabId)}
                            onPinChange={() => handlePinChange(tabId)}
                        />
                    );
                })}
                {pinnedTabIds.length > 0 && <div className="pinned-tab-spacer" />}
                {regularTabIds.map((tabId, i) => {
                    const idx = pinnedTabIds.length + i;
                    const isActive = tabId === activeTabId;
                    const isBeforeActive = idx === activeIndex - 1;
                    return (
                        <DroppableTab
                            key={tabId}
                            tabId={tabId}
                            workspaceId={workspace.oid}
                            activeTabId={activeTabId}
                            isActive={isActive}
                            isFirst={pinnedTabIds.length === 0 && i === 0}
                            isBeforeActive={isBeforeActive}
                            isPinned={false}
                            allTabCount={allTabIds.length}
                            tabIndex={idx}
                            onSelect={() => handleSelect(tabId)}
                            onClose={() => handleClose(tabId)}
                            onPinChange={() => handlePinChange(tabId)}
                        />
                    );
                })}
                <NewTabDropZone workspaceId={workspace.oid} />
            </div>
        </div>
    );
});

TabBar.displayName = "TabBar";

export { TabBar };
