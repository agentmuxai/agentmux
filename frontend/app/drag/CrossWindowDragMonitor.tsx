// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CrossWindowDragMonitor
 *
 * Monitors active react-dnd drags and handles cross-window operations.
 * When a drag ends without dropping on a valid target (didDrop=false),
 * the monitor checks the cursor position against all open windows.
 *
 * - If cursor is over another window: transfers the item to that window.
 * - If cursor is outside all windows: tears off into a new window.
 * - If same window: does nothing (react-dnd handled it or it was cancelled).
 *
 * This component must be rendered inside the <DndProvider>.
 */

import { atoms, getApi, globalStore } from "@/store/global";
import { WorkspaceService } from "@/app/store/services";
import { fireAndForget } from "@/util/util";
import { memo, useEffect, useRef } from "react";
import { useDragLayer } from "react-dnd";
import { tileItemType } from "@/layout/index";
import { tabItemType } from "@/app/tab/tabbar";
import type { LayoutNode } from "@/layout/index";

const CrossWindowDragMonitor = memo(() => {
    const windowLabelRef = useRef<string | null>(null);
    // Save last drag state before isDragging goes false
    const lastDragRef = useRef<{
        itemType: string | symbol | null;
        item: any;
    }>({ itemType: null, item: null });
    const prevDraggingRef = useRef(false);

    // Cache window label on mount
    useEffect(() => {
        getApi()
            .getWindowLabel()
            .then((label) => {
                windowLabelRef.current = label;
            });
    }, []);

    const { isDragging, itemType, item } = useDragLayer((monitor) => ({
        isDragging: monitor.isDragging(),
        itemType: monitor.getItemType(),
        item: monitor.getItem(),
    }));

    // Save current drag info whenever we're dragging
    useEffect(() => {
        if (isDragging && itemType && item) {
            lastDragRef.current = { itemType, item };
        }
    }, [isDragging, itemType, item]);

    // Detect drag end: isDragging transitions from true to false
    useEffect(() => {
        if (prevDraggingRef.current && !isDragging) {
            const { itemType: savedType, item: savedItem } = lastDragRef.current;
            if (savedType && savedItem) {
                // Delay slightly to let react-dnd process the drop first
                setTimeout(() => {
                    fireAndForget(() => handleDragEnd(savedType, savedItem, windowLabelRef.current));
                }, 50);
            }
            // Clear saved state
            lastDragRef.current = { itemType: null, item: null };
        }
        prevDraggingRef.current = isDragging;
    }, [isDragging]);

    return null; // Renderless component
});

CrossWindowDragMonitor.displayName = "CrossWindowDragMonitor";

/**
 * Main handler for when a drag ends. Checks if the drop target is in
 * another window and performs the appropriate cross-window operation.
 */
async function handleDragEnd(
    dragItemType: string | symbol,
    dragItem: any,
    sourceWindow: string | null
) {
    // Only handle known drag types
    const typeStr = String(dragItemType);
    if (typeStr !== tileItemType && typeStr !== tabItemType) return;

    // Get cursor position via Tauri (async)
    let cursorPoint: { x: number; y: number };
    try {
        const { invoke } = await import("@tauri-apps/api/core");
        cursorPoint = await invoke<{ x: number; y: number }>("get_cursor_point");
    } catch {
        return;
    }

    // Only proceed for multi-window scenarios
    let windows: string[];
    try {
        windows = await getApi().listWindows();
    } catch {
        return;
    }
    if (windows.length <= 1) return;

    const api = getApi();
    const src = sourceWindow ?? "main";
    const workspace = globalStore.get(atoms.workspace);
    const activeTabId = globalStore.get(atoms.activeTabId);
    if (!workspace) return;

    // Build payload from drag item
    let payload: { blockId?: string; tabId?: string };
    let dragType: "pane" | "tab";

    if (typeStr === tileItemType) {
        const node = dragItem as LayoutNode;
        const blockId = node?.data?.blockId;
        if (!blockId) return;
        payload = { blockId };
        dragType = "pane";
    } else {
        const tabId = dragItem?.tabId;
        if (!tabId) return;
        payload = { tabId };
        dragType = "tab";
    }

    try {
        // Use Tauri cross-drag infrastructure for hit-testing
        const dragId = await api.startCrossDrag(dragType, src, workspace.oid, activeTabId, payload);
        const targetWindow = await api.updateCrossDrag(dragId, cursorPoint.x, cursorPoint.y);

        if (targetWindow && targetWindow !== src) {
            // Cross-window drop
            await performCrossWindowDrop(dragType, payload, workspace.oid, activeTabId);
            await api.completeCrossDrag(dragId, targetWindow, cursorPoint.x, cursorPoint.y);
        } else if (!targetWindow) {
            // Tear-off (outside all windows)
            await performTearOff(dragType, payload, workspace.oid, activeTabId, cursorPoint.x, cursorPoint.y);
            await api.completeCrossDrag(dragId, null, cursorPoint.x, cursorPoint.y);
        } else {
            // Same window — no cross-window action needed
            await api.cancelCrossDrag(dragId);
        }
    } catch (e) {
        console.error("[cross-drag] Error:", e);
    }
}

/**
 * Transfer a pane or tab to another window's workspace.
 * Creates a new workspace for the item; the target window can then adopt it.
 */
async function performCrossWindowDrop(
    dragType: "pane" | "tab",
    payload: { blockId?: string; tabId?: string },
    sourceWsId: string,
    sourceTabId: string
) {
    if (dragType === "pane" && payload.blockId) {
        await WorkspaceService.TearOffBlock(payload.blockId, sourceTabId, sourceWsId, true);
    } else if (dragType === "tab" && payload.tabId) {
        await WorkspaceService.TearOffTab(payload.tabId, sourceWsId);
    }
}

/**
 * Tear off a pane or tab into a new window at the cursor position.
 */
async function performTearOff(
    dragType: "pane" | "tab",
    payload: { blockId?: string; tabId?: string },
    sourceWsId: string,
    sourceTabId: string,
    screenX: number,
    screenY: number
) {
    const api = getApi();

    if (dragType === "pane" && payload.blockId) {
        const newWsId = await WorkspaceService.TearOffBlock(
            payload.blockId,
            sourceTabId,
            sourceWsId,
            true
        );
        if (newWsId) {
            await api.openWindowAtPosition(screenX, screenY);
        }
    } else if (dragType === "tab" && payload.tabId) {
        const newWsId = await WorkspaceService.TearOffTab(payload.tabId, sourceWsId);
        if (newWsId) {
            await api.openWindowAtPosition(screenX, screenY);
        }
    }
}

export { CrossWindowDragMonitor };
