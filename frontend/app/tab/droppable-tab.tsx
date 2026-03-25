// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Logger } from "@/util/logger";
import { draggable } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { createMemo, onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import clsx from "clsx";
import { Tab } from "./tab";
import {
    tabItemType,
    GAP_PX,
    globalDragTabId,
    setGlobalDragTabId,
    insertionPoint,
    setInsertionPoint,
    bouncingTabId,
    tabWrapperRefs,
} from "./tabbar-dnd";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { getApi } from "@/store/global";
import { createSignal } from "solid-js";

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
    pinnedTabIds: string[];
    regularTabIds: string[];
    onSelect: () => void;
    onClose: () => void;
    onPinChange: () => void;
}

export function DroppableTab(props: DroppableTabProps): JSX.Element {
    let tabWrapRef!: HTMLDivElement;
    const [isDragging, setIsDragging] = createSignal(false);

    // Gap before (left padding) — this tab is the afterTabId of the insertion point
    const gapBefore = createMemo(() => {
        const ip = insertionPoint();
        return ip?.afterTabId === props.tabId ? GAP_PX : 0;
    });

    // Gap after (right padding) — this tab is the beforeTabId of the insertion point
    const gapAfter = createMemo(() => {
        const ip = insertionPoint();
        return ip?.beforeTabId === props.tabId ? GAP_PX : 0;
    });

    const isBouncing = () => bouncingTabId() === props.tabId;

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
                setInsertionPoint(null);
                setIsDragging(true);
                setCurrentDragPayload({ kind: "tab", tabId: props.tabId, workspaceId: props.workspaceId, isPinned: props.isPinned });
                getApi().setJsDragActive(true).catch(() => {});
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
                getApi().setJsDragActive(false).catch(() => {});
                // Do NOT clear currentDragPayload here — this fires for ALL drops including
                // out-of-window. Payload is cleared in the monitorForElements onDrop in
                // tabbar.tsx (only fires for valid in-window drops) so the CrossWindowDragMonitor
                // can still read it when dragend fires for out-of-window drops.
            },
        });

        // Set effectAllowed = "copy" so Windows OLE shows the plus-sign cursor
        // when dragging outside the WebView2 window (signals tearoff intent).
        // Atlaskit registers its dragstart handler on document in capture phase,
        // so this bubble-phase listener fires after Atlaskit has committed the
        // drag — effectAllowed is still writable until dragstart returns.
        const handleNativeDragStart = (e: DragEvent) => {
            if (e.dataTransfer) e.dataTransfer.effectAllowed = "copy";
        };
        tabWrapRef.addEventListener("dragstart", handleNativeDragStart);

        onCleanup(() => {
            tabWrapRef.removeEventListener("dragstart", handleNativeDragStart);
            tabWrapperRefs.delete(props.tabId);
            cleanupDraggable();
        });
    });

    return (
        <div
            ref={tabWrapRef!}
            data-tauri-drag-region="false"
            class={clsx("tab-drop-wrapper", {
                "tab-dragging": isDragging(),
                "tab-bouncing": isBouncing(),
            })}
            style={{
                "padding-left": `${gapBefore()}px`,
                "padding-right": `${gapAfter()}px`,
            } as JSX.CSSProperties}
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
