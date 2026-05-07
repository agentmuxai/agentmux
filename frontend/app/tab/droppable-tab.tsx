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
import { setTabGrabOffset } from "./tab-grab-offset";

export interface DroppableTabProps {
    tabId: string;
    workspaceId: string;
    activeTabId: string;
    isActive: boolean;
    isFirst: boolean;
    isBeforeActive: boolean;
    allTabCount: number;
    tabIndex: number;      // index into tabIds — used for activeIndex math and ReorderTab
    tabIds: string[];
    onSelect: () => void;
    onClose: () => void;
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
                tabIndex: props.tabIndex,
                type: tabItemType,
            }),
            onGenerateDragPreview: ({ location, source }) => {
                // Capture grab offset here (rather than onDragStart)
                // because pragmatic-dnd's DragLocation is on this event;
                // onDragStart only carries `source`. Used by tear-off to
                // anchor the new window so the cursor stays on the same
                // pixel of the same tab across the handoff.
                //
                // Note: we DO NOT suppress the OS drag image even though
                // the spec drafted it. SC_MOVE doesn't actually engage
                // during the HTML5 drag (pragmatic-dnd's OLE capture
                // blocks the modal move-loop), so the new window only
                // appears on mouseup. Suppressing the OS ghost leaves
                // the user with a no-drop cursor and zero visual
                // feedback during drag — strictly worse. Spec §4.5
                // updated to reflect this.
                const tabRect = tabWrapRef.getBoundingClientRect();
                setTabGrabOffset({
                    x: location.current.input.clientX - tabRect.left,
                    y: location.current.input.clientY - tabRect.top,
                });
                Logger.info("dnd", "tab-drag preview generated", {
                    tabId: source.data.tabId,
                    grabX: location.current.input.clientX - tabRect.left,
                    grabY: location.current.input.clientY - tabRect.top,
                });
            },
            onDragStart: () => {
                setGlobalDragTabId(props.tabId);
                setInsertionPoint(null);
                setIsDragging(true);
                setCurrentDragPayload({ kind: "tab", tabId: props.tabId, workspaceId: props.workspaceId });
                getApi().setJsDragActive(true).catch(() => {});
                Logger.info("dnd", "tab-drag started", {
                    tabId: props.tabId,
                    workspaceId: props.workspaceId,
                    tabIndex: props.tabIndex,
                });
            },
            onDrop: () => {
                setGlobalDragTabId(null);
                setIsDragging(false);
                setTabGrabOffset(null);
                getApi().setJsDragActive(false).catch(() => {});
                // Do NOT clear currentDragPayload here — this fires for ALL drops including
                // out-of-window. Payload is cleared in the monitorForElements onDrop in
                // tabbar.tsx (only fires for valid in-window drops) so the CrossWindowDragMonitor
                // can still read it when dragend fires for out-of-window drops.
            },
        });

        onCleanup(() => {
            tabWrapperRefs.delete(props.tabId);
            cleanupDraggable();
        });
    });

    return (
        <div
            ref={tabWrapRef!}
            data-drag-region="false"
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
                onSelect={props.onSelect}
                onClose={props.onClose}
                onDragStart={() => {}}
                onLoaded={() => {}}
            />
        </div>
    );
}
