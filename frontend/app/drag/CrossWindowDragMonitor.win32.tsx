// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CrossWindowDragMonitor — Windows/WebView2
 *
 * On Windows, OLE may not deliver `dragend` back to the WebView2 source when
 * the mouse is released over a native app (Explorer, VS Code, etc.). This file
 * adds a fallback that detects the end of such drags without a `dragend` event.
 *
 * Strategy:
 *   1. On `dragleave` (cursor left our window), arm an 800ms fallback timer.
 *   2. On `dragenter` (cursor returned), cancel the timer — user is still dragging.
 *   3. On `dragend` (normal completion), cancel the timer.
 *   4. When the timer fires, call `get_mouse_button_state` (Win32 GetAsyncKeyState).
 *      - If mouse button still pressed → user is hovering, not dropping. Reschedule.
 *      - If released → mouse was released outside; trigger tearoff.
 *
 * This avoids the previous `drag`-heartbeat approach that fired whenever the cursor
 * was over any native app — even during an active hover with no drop intended.
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
        Logger.debug("dnd:cross", "CrossWindowDragMonitor mounted (win32)", { windowLabel: windowLabelRef });

        let fallbackTimer: ReturnType<typeof setTimeout> | null = null;

        const clearFallback = () => {
            if (fallbackTimer !== null) {
                clearTimeout(fallbackTimer);
                fallbackTimer = null;
            }
        };

        const checkAndFireFallback = async () => {
            fallbackTimer = null;
            const payload = _currentDragPayload;
            if (!payload) return;

            // Query Windows directly: is the left mouse button still held?
            let isButtonPressed = false;
            try {
                const { invoke } = await import("@tauri-apps/api/core");
                isButtonPressed = await invoke<boolean>("get_mouse_button_state");
            } catch (e) {
                // If the call fails, be conservative and reschedule rather than
                // triggering a spurious tearoff.
                Logger.warn("dnd:cross", "get_mouse_button_state failed, rescheduling", { error: String(e) });
                fallbackTimer = setTimeout(checkAndFireFallback, 800);
                return;
            }

            if (isButtonPressed) {
                // User is still holding the button — just hovering, not dropping.
                Logger.debug("dnd:cross", "fallback: button still pressed, rescheduling");
                fallbackTimer = setTimeout(checkAndFireFallback, 800);
                return;
            }

            // Button released outside our window — OLE didn't deliver dragend.
            _currentDragPayload = null;
            Logger.info("dnd:cross", "drag fallback fired: button released outside window (OLE dragend not received)");
            getApi().releaseDragCapture().catch(() => {});
            getApi().restoreDragCursor().catch(() => {});
            await handleCrossWindowDragEnd(payload, windowLabelRef);
        };

        // Cursor left our WebView2 window — arm the fallback.
        const handleDragLeave = (e: DragEvent) => {
            if (e.relatedTarget !== null) return; // just moved to another element inside us
            if (!_currentDragPayload) return;
            if (fallbackTimer === null) {
                Logger.debug("dnd:cross", "dragleave (outside window) — arming fallback timer");
                fallbackTimer = setTimeout(checkAndFireFallback, 800);
            }
        };

        // Cursor re-entered our window — cancel the fallback.
        const handleDragEnter = (e: DragEvent) => {
            if (e.relatedTarget !== null) return; // internal element transition, not a re-entry
            if (fallbackTimer !== null) {
                Logger.debug("dnd:cross", "dragenter (back in window) — cancelling fallback timer");
                clearFallback();
            }
        };

        const handleDragEnd = async (e: DragEvent) => {
            clearFallback();
            const payload = _currentDragPayload;
            _currentDragPayload = null;

            Logger.info("dnd:cross", "dragend fired", { hasPayload: !!payload, dropEffect: e.dataTransfer?.dropEffect });

            if (!payload) return;

            // Release WebView2 mouse capture immediately — IDropSource may leave it active
            // after an out-of-window HTML5 drag, breaking subsequent mousedown delivery.
            getApi().releaseDragCapture().catch(() => {});
            getApi().restoreDragCursor().catch(() => {});

            await new Promise((r) => setTimeout(r, 50));
            await handleCrossWindowDragEnd(payload, windowLabelRef);
        };

        document.addEventListener("dragleave", handleDragLeave);
        document.addEventListener("dragenter", handleDragEnter);
        document.addEventListener("dragend", handleDragEnd);
        onCleanup(() => {
            document.removeEventListener("dragleave", handleDragLeave);
            document.removeEventListener("dragenter", handleDragEnter);
            document.removeEventListener("dragend", handleDragEnd);
            clearFallback();
        });
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
