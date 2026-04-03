// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CrossWindowDragMonitor — Linux
 *
 * On Linux, WebKitGTK reliably delivers `dragend` to the source element even when
 * the drop occurs over a native app. No OLE fallback needed.
 */

import { atoms, getApi } from "@/store/global";
import { WorkspaceService } from "@/app/store/services";
import { Logger } from "@/util/logger";
import { invokeCommand } from "@/app/platform/ipc";
import { onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import type { LayoutNode } from "@/layout/lib/types";

export type DragItemPayload =
    | { kind: "tile"; node: LayoutNode }
    | { kind: "tab"; tabId: string; workspaceId: string; isPinned: boolean };

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
        Logger.debug("dnd:cross", "CrossWindowDragMonitor mounted (linux)", { windowLabel: windowLabelRef });

        const handleDragEnd = async (e: DragEvent) => {
            const payload = _currentDragPayload;
            _currentDragPayload = null;

            Logger.info("dnd:cross", "dragend fired", { hasPayload: !!payload, dropEffect: e.dataTransfer?.dropEffect });

            if (!payload) return;

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
        cursorPoint = await invokeCommand<{ x: number; y: number }>("get_cursor_point");
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
) {}

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
        if (newWsId) await api.openWindowAtPosition(screenX, screenY, newWsId);
    } else if (dragType === "tab" && payload.tabId) {
        const newWsId = await WorkspaceService.TearOffTab(payload.tabId, sourceWsId);
        if (newWsId) await api.openWindowAtPosition(screenX, screenY, newWsId);
    }
}

export { CrossWindowDragMonitor };
