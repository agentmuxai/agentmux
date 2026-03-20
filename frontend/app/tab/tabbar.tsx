// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, createTab, setActiveTab } from "@/store/global";
import { Logger } from "@/util/logger";
import { fireAndForget } from "@/util/util";
import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { draggable, dropTargetForElements, monitorForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { createSignal, For, onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import clsx from "clsx";
import { WorkspaceService } from "../store/services";
import { deleteLayoutModelForTab } from "@/layout/index";
import { Tab } from "./tab";
import "./tabbar.scss";

export const tabItemType = "TAB_ITEM";

/**
 * Computes the insertion index for ReorderTab, accounting for the backend's
 * remove-then-insert behaviour. When the source tab is before the target in
 * the array, the array shrinks by 1 before insertion, so the raw index must
 * be decremented by 1 to land in the correct slot.
 */
function computeInsertIndex(sourceIndex: number, targetIndex: number, side: "left" | "right"): number {
    const rawIndex = side === "left" ? targetIndex : targetIndex + 1;
    return sourceIndex < rawIndex ? rawIndex - 1 : rawIndex;
}

interface TabBarProps {
    workspace: Workspace;
}

// Module-level drag state so drop targets and the monitor can coordinate.
let globalDragTabId: string | null = null;

// Nearest-tab hint: when the cursor is outside all drop targets, the monitor
// sets this to { tabId, side } so the nearest DroppableTab shows an indicator.
const [nearestHint, setNearestHint] = createSignal<{ tabId: string; side: "left" | "right" } | null>(null);

// Registry of tab wrapper elements for nearest-tab computation.
const tabWrapperRefs = new Map<string, HTMLDivElement>();

function computeNearestTab(clientX: number, clientY: number): { tabId: string; side: "left" | "right" } | null {
    let bestTabId: string | null = null;
    let bestDist = Infinity;
    let bestSide: "left" | "right" = "left";

    for (const [tabId, el] of tabWrapperRefs) {
        if (tabId === globalDragTabId) continue;
        const rect = el.getBoundingClientRect();
        const midX = rect.left + rect.width / 2;
        const dist = Math.abs(clientX - midX);
        if (dist < bestDist) {
            bestDist = dist;
            bestTabId = tabId;
            bestSide = clientX < midX ? "left" : "right";
        }
    }
    if (!bestTabId) return null;
    return { tabId: bestTabId, side: bestSide };
}

/**
 * Wraps a Tab component with pragmatic-dnd drag/drop for tab reordering.
 * When the cursor is directly over this tab, uses drop target events.
 * When the cursor is outside all tabs, reads the nearestHint signal.
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
    const [isDragging, setIsDragging] = createSignal(false);
    const [directInsertSide, setDirectInsertSide] = createSignal<"left" | "right" | null>(null);
    const [isDirectTarget, setIsDirectTarget] = createSignal(false);

    // Effective insert side: direct drop target takes priority, otherwise use nearest hint
    const insertSide = (): "left" | "right" | null => {
        if (isDragging()) return null;
        if (isDirectTarget()) return directInsertSide();
        const hint = nearestHint();
        if (hint && hint.tabId === props.tabId) return hint.side;
        return null;
    };

    onMount(() => {
        if (!tabWrapRef) return;

        // Register in the ref map for nearest-tab computation
        tabWrapperRefs.set(props.tabId, tabWrapRef);

        // Register as draggable
        const cleanupDraggable = draggable({
            element: tabWrapRef,
            canDrag: () => props.allTabCount > 1,
            getInitialData: () => ({
                tabId: props.tabId,
                workspaceId: props.workspaceId,
                isPinned: props.isPinned,
                tabIndex: props.tabIndex,
                type: tabItemType,
            }),
            onDragStart: () => {
                globalDragTabId = props.tabId;
                setIsDragging(true);
                Logger.info("dnd", "tab-drag started", {
                    tabId: props.tabId,
                    workspaceId: props.workspaceId,
                    isPinned: props.isPinned,
                });
            },
            onDrop: () => {
                globalDragTabId = null;
                setIsDragging(false);
                setNearestHint(null);
            },
        });

        // Register as drop target
        const cleanupDropTarget = dropTargetForElements({
            element: tabWrapRef,
            canDrop: ({ source }) =>
                source.data.type === tabItemType && source.data.tabId !== props.tabId,
            onDragEnter: ({ location }) => {
                setIsDirectTarget(true);
                const rect = tabWrapRef.getBoundingClientRect();
                const midX = rect.left + rect.width / 2;
                setDirectInsertSide(location.current.input.clientX < midX ? "left" : "right");
            },
            onDrag: ({ location }) => {
                const rect = tabWrapRef.getBoundingClientRect();
                const midX = rect.left + rect.width / 2;
                setDirectInsertSide(location.current.input.clientX < midX ? "left" : "right");
            },
            onDragLeave: () => {
                setIsDirectTarget(false);
                setDirectInsertSide(null);
            },
            onDrop: ({ source, location }) => {
                setIsDirectTarget(false);
                setDirectInsertSide(null);
                setNearestHint(null);
                const draggedTabId = source.data.tabId as string;
                if (draggedTabId === props.tabId) return;

                const rect = tabWrapRef.getBoundingClientRect();
                const midX = rect.left + rect.width / 2;
                const side = location.current.input.clientX < midX ? "left" : "right";
                const sourceIndex = source.data.tabIndex as number;
                const newIndex = computeInsertIndex(sourceIndex, props.tabIndex, side);

                Logger.info("dnd", "tab-reorder drop", {
                    draggedTabId,
                    targetTabId: props.tabId,
                    side,
                    sourceIndex,
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
            },
        });

        onCleanup(() => {
            tabWrapperRefs.delete(props.tabId);
            cleanupDraggable();
            cleanupDropTarget();
        });
    });

    return (
        <div
            ref={tabWrapRef!}
            class={clsx("tab-drop-wrapper", {
                "tab-dragging": isDragging(),
                "tab-insert-left": insertSide() === "left",
                "tab-insert-right": insertSide() === "right",
            })}
        >
            <Tab
                id={props.tabId}
                active={props.isActive}
                isFirst={props.isFirst}
                isBeforeActive={props.isBeforeActive}
                isDragging={isDragging()}
                tabWidth={0}
                isNew={false}
                isPinned={props.isPinned}
                onSelect={props.onSelect}
                onClose={props.onClose}
                onDragStart={() => {}}
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

    const { dragProps } = useWindowDrag();

    // Global drag monitor: when cursor leaves all tab drop targets during a
    // tab drag, compute the nearest tab and set the insertion hint so the
    // indicator stays visible even when dragging outside the tab bar.
    onMount(() => {
        const cleanup = monitorForElements({
            canMonitor: ({ source }) => source.data.type === tabItemType,
            onDrag: ({ location }) => {
                // If cursor is over a drop target, that target handles its own indicator
                if (location.current.dropTargets.length > 0) {
                    setNearestHint(null);
                    return;
                }
                // Cursor is outside all drop targets — find nearest tab
                const hint = computeNearestTab(
                    location.current.input.clientX,
                    location.current.input.clientY
                );
                setNearestHint(hint);
            },
            onDrop: () => {
                // If dropped outside all targets, execute the reorder to nearest position
                const hint = nearestHint();
                if (hint && globalDragTabId && globalDragTabId !== hint.tabId) {
                    const el = tabWrapperRefs.get(hint.tabId);
                    if (el) {
                        // Find the tab index from the element's position in the tab bar
                        const allTabs = allTabIds();
                        const targetIdx = allTabs.indexOf(hint.tabId);
                        const sourceIdx = allTabs.indexOf(globalDragTabId);
                        if (targetIdx >= 0 && sourceIdx >= 0) {
                            const newIndex = computeInsertIndex(sourceIdx, targetIdx, hint.side);
                            const draggedTabId = globalDragTabId;
                            const wsId = props.workspace?.oid;
                            Logger.info("dnd", "tab-reorder drop (nearest)", {
                                draggedTabId,
                                targetTabId: hint.tabId,
                                side: hint.side,
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
            <button class="add-tab-btn" onClick={handleAddTab} title="New Tab" data-tauri-drag-region="false">
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
