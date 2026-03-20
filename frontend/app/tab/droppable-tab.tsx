// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Logger } from "@/util/logger";
import { fireAndForget } from "@/util/util";
import { draggable, dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { createSignal, onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import clsx from "clsx";
import { WorkspaceService } from "../store/services";
import { Tab } from "./tab";
import {
    tabItemType,
    globalDragTabId,
    setGlobalDragTabId,
    nearestHint,
    setNearestHint,
    tabWrapperRefs,
    computeInsertIndex,
} from "./tabbar-dnd";

export interface DroppableTabProps {
    tabId: string;
    workspaceId: string;
    activeTabId: string;
    isActive: boolean;
    isFirst: boolean;
    isBeforeActive: boolean;
    isPinned: boolean;
    allTabCount: number;
    tabIndex: number;      // combined index (pinned + regular) — used for activeIndex math only
    sectionIndex: number;  // index within its section — what the backend expects for ReorderTab
    onSelect: () => void;
    onClose: () => void;
    onPinChange: () => void;
}

export function DroppableTab(props: DroppableTabProps): JSX.Element {
    let tabWrapRef!: HTMLDivElement;
    const [isDragging, setIsDragging] = createSignal(false);
    const [directInsertSide, setDirectInsertSide] = createSignal<"left" | "right" | null>(null);
    const [isDirectTarget, setIsDirectTarget] = createSignal(false);

    // Effective insert side: direct drop target takes priority, otherwise nearest hint
    const insertSide = (): "left" | "right" | null => {
        if (isDragging()) return null;
        if (isDirectTarget()) return directInsertSide();
        const hint = nearestHint();
        if (hint && hint.tabId === props.tabId) return hint.side;
        return null;
    };

    onMount(() => {
        if (!tabWrapRef) return;

        tabWrapperRefs.set(props.tabId, tabWrapRef);

        const cleanupDraggable = draggable({
            element: tabWrapRef,
            canDrag: () => props.allTabCount > 1,
            getInitialData: () => ({
                tabId: props.tabId,
                workspaceId: props.workspaceId,
                isPinned: props.isPinned,
                tabIndex: props.tabIndex,
                sectionIndex: props.sectionIndex,
                type: tabItemType,
            }),
            onDragStart: () => {
                setGlobalDragTabId(props.tabId);
                setIsDragging(true);
                Logger.info("dnd", "tab-drag started", {
                    tabId: props.tabId,
                    workspaceId: props.workspaceId,
                    isPinned: props.isPinned,
                    sectionIndex: props.sectionIndex,
                });
            },
            onDrop: () => {
                setGlobalDragTabId(null);
                setIsDragging(false);
                setNearestHint(null);
            },
        });

        const cleanupDropTarget = dropTargetForElements({
            element: tabWrapRef,
            canDrop: ({ source }) =>
                source.data.type === tabItemType && source.data.tabId !== props.tabId,
            onDragEnter: ({ location }) => {
                setIsDirectTarget(true);
                const rect = tabWrapRef.getBoundingClientRect();
                setDirectInsertSide(location.current.input.clientX < rect.left + rect.width / 2 ? "left" : "right");
            },
            onDrag: ({ location }) => {
                const rect = tabWrapRef.getBoundingClientRect();
                setDirectInsertSide(location.current.input.clientX < rect.left + rect.width / 2 ? "left" : "right");
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
                // Cross-section drops (pinned ↔ regular) not yet supported
                if ((source.data.isPinned as boolean) !== props.isPinned) return;

                const rect = tabWrapRef.getBoundingClientRect();
                const side = location.current.input.clientX < rect.left + rect.width / 2 ? "left" : "right";
                const sourceSectionIndex = source.data.sectionIndex as number;
                const newIndex = computeInsertIndex(sourceSectionIndex, props.sectionIndex, side);

                Logger.info("dnd", "tab-reorder drop", {
                    draggedTabId,
                    targetTabId: props.tabId,
                    side,
                    sourceSectionIndex,
                    targetSectionIndex: props.sectionIndex,
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
            data-tauri-drag-region="false"
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
