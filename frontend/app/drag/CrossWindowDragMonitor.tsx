// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CrossWindowDragMonitor
 *
 * Monitors active drags and handles cross-window operations.
 * When a drag ends without dropping on a valid target, the monitor
 * checks the cursor position against all open windows.
 *
 * - If cursor is over another window: transfers the item to that window.
 * - If cursor is outside all windows: tears off into a new window.
 * - If same window: does nothing.
 *
 * react-dnd has been removed. This component now hooks into the native
 * HTML5 drag lifecycle via document-level dragend events.
 */

import { atoms, getApi, globalStore } from "@/store/global";
import { WorkspaceService } from "@/app/store/services";
import { Logger } from "@/util/logger";
import { onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import type { LayoutNode } from "@/layout/lib/types";

// Shared drag state set by TileLayout / TabBar drag handlers
export type DragItemPayload =
    | { kind: "tile"; node: LayoutNode }
    | { kind: "tab"; tabId: string; workspaceId: string; isPinned: boolean };

// Module-level drag state so TileLayout/TabBar can set it before dragend fires
let _currentDragPayload: DragItemPayload | null = null;

export function setCurrentDragPayload(payload: DragItemPayload | null) {
    _currentDragPayload = payload;
}

export function getCurrentDragPayload(): DragItemPayload | null {
    return _currentDragPayload;
}

function CrossWindowDragMonitor(): JSX.Element {
    let windowLabelRef: string | null = null;

    onMount(async () => {
        windowLabelRef = await getApi().getWindowLabel();
        Logger.debug("dnd:cross", "CrossWindowDragMonitor mounted", { windowLabel: windowLabelRef });

        const handleDragEnd = async (e: DragEvent) => {
            const payload = _currentDragPayload;
            _currentDragPayload = null;

            Logger.info("dnd:cross", "dragend fired", { hasPayload: !!payload, dropEffect: e.dataTransfer?.dropEffect });

            if (!payload) return;

            // Fire-and-forget: release WebView2 mouse capture immediately on dragend,
            // before the 50ms delay. After an HTML5 drag that ended outside the window,
            // WebView2's OLE IDropSource may leave capture active, preventing the next
            // mousedown from being delivered correctly to Tauri's drag-region JS handler.
            // ReleaseCapture + WM_CANCELMODE to all child HWNDs resets Chromium's pointer state.
            getApi().releaseDragCapture().catch(() => {});

            // Brief delay to allow native drop handlers to run first
            await new Promise((r) => setTimeout(r, 50));
            await handleCrossWindowDragEnd(payload, windowLabelRef);

        };

        document.addEventListener("dragend", handleDragEnd);
        onCleanup(() => document.removeEventListener("dragend", handleDragEnd));
    });

    return null;
}

async function handleCrossWindowDragEnd(payload: DragItemPayload, sourceWindow: string | null) {
    let cursorPoint: { x: number; y: number };
    try {
        const { invoke } = await import("@tauri-apps/api/core");
        cursorPoint = await invoke<{ x: number; y: number }>("get_cursor_point");
    } catch (e) {
        Logger.error("dnd:cross", "failed to get cursor position", { error: String(e) });
        return;
    }

    let windows: string[];
    try {
        windows = await getApi().listWindows();
    } catch (e) {
        Logger.error("dnd:cross", "failed to list windows", { error: String(e) });
        return;
    }

    const api = getApi();
    const src = sourceWindow ?? "main";
    const workspace = atoms.workspace() as Workspace | undefined;
    const activeTabId = atoms.activeTabId();

    if (!workspace) {
        Logger.warn("dnd:cross", "no workspace found — aborting cross-window drag");
        return;
    }

    let dragPayloadForApi: { blockId?: string; tabId?: string };
    let dragType: "pane" | "tab";

    if (payload.kind === "tile") {
        const blockId = payload.node?.data?.blockId;
        if (!blockId) return;
        dragPayloadForApi = { blockId };
        dragType = "pane";
    } else {
        dragPayloadForApi = { tabId: payload.tabId };
        dragType = "tab";
    }

    try {
        const dragId = await api.startCrossDrag(dragType, src, workspace.oid, activeTabId, dragPayloadForApi);
        const targetWindow = await api.updateCrossDrag(dragId, cursorPoint.x, cursorPoint.y);

        if (targetWindow && targetWindow !== src) {
            await performCrossWindowDrop(dragType, dragPayloadForApi, workspace.oid, activeTabId);
            await api.completeCrossDrag(dragId, targetWindow, cursorPoint.x, cursorPoint.y);
        } else if (!targetWindow) {
            await performTearOff(dragType, dragPayloadForApi, workspace.oid, activeTabId, cursorPoint.x, cursorPoint.y);
            await api.completeCrossDrag(dragId, null, cursorPoint.x, cursorPoint.y);
            // After an out-of-window drop, WebView2 may retain mouse capture from the
            // Chromium IDropSource operation (OS delivered mouseup outside the window).
            // Post WM_CANCELMODE to our HWND to release any capture and restore normal
            // pointer state so title-bar dragging (data-tauri-drag-region) works again.
            try { await api.releaseDragCapture(); } catch {}
        } else {
            await api.cancelCrossDrag(dragId);
        }
    } catch (e) {
        Logger.error("dnd:cross", "cross-window drag error", { error: String(e), dragType, dragPayloadForApi });
    }
}

async function performCrossWindowDrop(
    _dragType: "pane" | "tab",
    _payload: { blockId?: string; tabId?: string },
    _sourceWsId: string,
    _sourceTabId: string
) {
    // The target window handles the actual move when it receives the cross-drag-end event.
    // Source window just completes the drag session so the event fires with targetWindow set.
}

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
        const newWsId = await WorkspaceService.TearOffBlock(payload.blockId, sourceTabId, sourceWsId, true);
        if (newWsId) {
            await api.openWindowAtPosition(screenX, screenY, newWsId);
        }
    } else if (dragType === "tab" && payload.tabId) {
        const newWsId = await WorkspaceService.TearOffTab(payload.tabId, sourceWsId);
        if (newWsId) {
            await api.openWindowAtPosition(screenX, screenY, newWsId);
        }
    }
}

export { CrossWindowDragMonitor };
