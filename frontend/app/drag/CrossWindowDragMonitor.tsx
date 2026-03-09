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
import { Logger } from "@/util/logger";
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
                Logger.debug("dnd:cross", "CrossWindowDragMonitor mounted", { windowLabel: label });
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
            Logger.debug("dnd:cross", "drag-layer active", { itemType: String(itemType), item });
        }
    }, [isDragging, itemType, item]);

    // Replace the system no-drop cursor with a crosshair while dragging
    useEffect(() => {
        if (isDragging) {
            const typeStr = itemType ? String(itemType) : "";
            if (typeStr === tileItemType || typeStr === tabItemType) {
                fireAndForget(async () => {
                    try {
                        await getApi().setDragCursor();
                    } catch (e) {
                        Logger.debug("dnd:cross", "setDragCursor failed (non-critical)", { error: String(e) });
                    }
                });
            }
        }
    }, [isDragging, itemType]);

    // Detect drag end: isDragging transitions from true to false
    useEffect(() => {
        if (prevDraggingRef.current && !isDragging) {
            // Restore system cursors immediately
            fireAndForget(async () => {
                try {
                    await getApi().restoreDragCursor();
                } catch (e) {
                    Logger.debug("dnd:cross", "restoreDragCursor failed (non-critical)", { error: String(e) });
                }
            });

            const { itemType: savedType, item: savedItem } = lastDragRef.current;
            Logger.info("dnd:cross", "drag ended — checking for cross-window", {
                hasItem: !!savedType,
                itemType: savedType ? String(savedType) : null,
            });
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
    if (typeStr !== tileItemType && typeStr !== tabItemType) {
        Logger.debug("dnd:cross", "ignoring unknown drag type", { type: typeStr });
        return;
    }

    // Get cursor position via Tauri (async)
    let cursorPoint: { x: number; y: number };
    try {
        const { invoke } = await import("@tauri-apps/api/core");
        cursorPoint = await invoke<{ x: number; y: number }>("get_cursor_point");
        Logger.debug("dnd:cross", "cursor position", { x: cursorPoint.x, y: cursorPoint.y });
    } catch (e) {
        Logger.error("dnd:cross", "failed to get cursor position", { error: String(e) });
        return;
    }

    // List open windows (used for logging; tear-off works even with 1 window)
    let windows: string[];
    try {
        windows = await getApi().listWindows();
    } catch (e) {
        Logger.error("dnd:cross", "failed to list windows", { error: String(e) });
        return;
    }

    const api = getApi();
    const src = sourceWindow ?? "main";
    const workspace = globalStore.get(atoms.workspace);
    const activeTabId = globalStore.get(atoms.activeTabId);
    if (!workspace) {
        Logger.warn("dnd:cross", "no workspace found — aborting cross-window drag");
        return;
    }

    // Build payload from drag item
    let payload: { blockId?: string; tabId?: string };
    let dragType: "pane" | "tab";

    if (typeStr === tileItemType) {
        const node = dragItem as LayoutNode;
        const blockId = node?.data?.blockId;
        if (!blockId) {
            Logger.warn("dnd:cross", "tile drag item has no blockId", { dragItem });
            return;
        }
        payload = { blockId };
        dragType = "pane";
    } else {
        const tabId = dragItem?.tabId;
        if (!tabId) {
            Logger.warn("dnd:cross", "tab drag item has no tabId", { dragItem });
            return;
        }
        payload = { tabId };
        dragType = "tab";
    }

    Logger.info("dnd:cross", "starting cross-window drag check", {
        dragType,
        payload,
        sourceWindow: src,
        workspaceId: workspace.oid,
        activeTabId,
        cursor: cursorPoint,
        windowCount: windows.length,
    });

    try {
        // Use Tauri cross-drag infrastructure for hit-testing
        const dragId = await api.startCrossDrag(dragType, src, workspace.oid, activeTabId, payload);
        Logger.debug("dnd:cross", "cross-drag session started", { dragId });

        const targetWindow = await api.updateCrossDrag(dragId, cursorPoint.x, cursorPoint.y);
        Logger.info("dnd:cross", "hit-test result", { dragId, targetWindow, sourceWindow: src });

        if (targetWindow && targetWindow !== src) {
            // Cross-window drop
            Logger.info("dnd:cross", "CROSS-WINDOW DROP", { dragType, targetWindow, payload });
            await performCrossWindowDrop(dragType, payload, workspace.oid, activeTabId);
            await api.completeCrossDrag(dragId, targetWindow, cursorPoint.x, cursorPoint.y);
            Logger.info("dnd:cross", "cross-window drop complete", { dragId, targetWindow });
        } else if (!targetWindow) {
            // Tear-off (outside all windows)
            Logger.info("dnd:cross", "TEAR-OFF (outside all windows)", { dragType, payload, cursor: cursorPoint });
            await performTearOff(dragType, payload, workspace.oid, activeTabId, cursorPoint.x, cursorPoint.y);
            await api.completeCrossDrag(dragId, null, cursorPoint.x, cursorPoint.y);
            Logger.info("dnd:cross", "tear-off complete", { dragId });
        } else {
            // Same window — no cross-window action needed
            Logger.debug("dnd:cross", "same window — cancelling cross-drag", { dragId, targetWindow });
            await api.cancelCrossDrag(dragId);
        }
    } catch (e) {
        Logger.error("dnd:cross", "cross-window drag error", { error: String(e), dragType, payload });
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
    Logger.info("dnd:cross", "performCrossWindowDrop", { dragType, payload, sourceWsId, sourceTabId });
    try {
        if (dragType === "pane" && payload.blockId) {
            const newWsId = await WorkspaceService.TearOffBlock(payload.blockId, sourceTabId, sourceWsId, true);
            Logger.info("dnd:cross", "TearOffBlock result", { blockId: payload.blockId, newWsId });
        } else if (dragType === "tab" && payload.tabId) {
            const newWsId = await WorkspaceService.TearOffTab(payload.tabId, sourceWsId);
            Logger.info("dnd:cross", "TearOffTab result", { tabId: payload.tabId, newWsId });
        }
    } catch (e) {
        Logger.error("dnd:cross", "performCrossWindowDrop failed", { dragType, payload, error: String(e) });
        throw e;
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
    Logger.info("dnd:cross", "performTearOff", { dragType, payload, sourceWsId, sourceTabId, screenX, screenY });

    try {
        if (dragType === "pane" && payload.blockId) {
            const newWsId = await WorkspaceService.TearOffBlock(
                payload.blockId,
                sourceTabId,
                sourceWsId,
                true
            );
            Logger.info("dnd:cross", "TearOffBlock for tear-off result", { blockId: payload.blockId, newWsId });
            if (newWsId) {
                const newLabel = await api.openWindowAtPosition(screenX, screenY);
                Logger.info("dnd:cross", "new window opened for pane tear-off", { newLabel, screenX, screenY });
            } else {
                Logger.warn("dnd:cross", "TearOffBlock returned no workspace — skipping window creation");
            }
        } else if (dragType === "tab" && payload.tabId) {
            const newWsId = await WorkspaceService.TearOffTab(payload.tabId, sourceWsId);
            Logger.info("dnd:cross", "TearOffTab for tear-off result", { tabId: payload.tabId, newWsId });
            if (newWsId) {
                const newLabel = await api.openWindowAtPosition(screenX, screenY);
                Logger.info("dnd:cross", "new window opened for tab tear-off", { newLabel, screenX, screenY });
            } else {
                Logger.warn("dnd:cross", "TearOffTab returned no workspace — skipping window creation");
            }
        }
    } catch (e) {
        Logger.error("dnd:cross", "performTearOff failed", { dragType, payload, error: String(e) });
        throw e;
    }
}

export { CrossWindowDragMonitor };
