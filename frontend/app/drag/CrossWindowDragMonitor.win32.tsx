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

import { atoms, getApi } from "@/store/global";
import { WorkspaceService } from "@/app/store/services";
import { getLayoutModelForStaticTab, LayoutTreeActionType, LayoutTreeDeleteNodeAction } from "@/layout/index";
import { invokeCommand } from "@/app/platform/ipc";
import { Logger } from "@/util/logger";
import { openTearOffWindow } from "./tear-off-pool-helper";
import { getTabGrabOffset } from "@/app/tab/tab-grab-offset";
import { onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import type { LayoutNode } from "@/layout/lib/types";

// Shared drag state set by TileLayout / TabBar drag handlers
export type DragItemPayload =
    | { kind: "tile"; node: LayoutNode }
    | { kind: "tab"; tabId: string; workspaceId: string };

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
                isButtonPressed = await invokeCommand<boolean>("get_mouse_button_state");
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

            // Snapshot the grab offset RIGHT NOW. pragmatic-dnd's onDrop
            // in droppable-tab.tsx clears it via setTabGrabOffset(null);
            // by the time the setTimeout below fires and performTearOff
            // calls getTabGrabOffset(), the offset is already null and
            // the tear-off anchor falls back to cursor-centered.
            const grabOffsetSnapshot = getTabGrabOffset();

            Logger.info("dnd:cross", "dragend fired", {
                hasPayload: !!payload,
                dropEffect: e.dataTransfer?.dropEffect,
                grabOffsetX: grabOffsetSnapshot?.x,
                grabOffsetY: grabOffsetSnapshot?.y,
            });

            if (!payload) return;

            // Release WebView2 mouse capture immediately — IDropSource may leave it active
            // after an out-of-window HTML5 drag, breaking subsequent mousedown delivery.
            getApi().releaseDragCapture().catch(() => {});

            await new Promise((r) => setTimeout(r, 50));
            await handleCrossWindowDragEnd(payload, windowLabelRef, grabOffsetSnapshot);
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

async function handleCrossWindowDragEnd(
    payload: DragItemPayload,
    sourceWindow: string | null,
    grabOffsetSnapshot: ReturnType<typeof getTabGrabOffset> = null,
) {
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
            await performTearOff(dragType, dragPayloadForApi, workspace.oid, activeTabId, cursorPoint.x, cursorPoint.y, grabOffsetSnapshot);
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

// Default floating-pane size when we don't have a captured source rect
// (the tearoff-pane-size spec — agenta/feat-tearoff-match-source-size —
// will refine this to use the source pane's measured dimensions). 720×480
// is a comfortable single-block working area without dominating the screen.
const DEFAULT_FLOATER_WIDTH = 720;
const DEFAULT_FLOATER_HEIGHT = 480;

async function performTearOff(
    dragType: "pane" | "tab",
    payload: { blockId?: string; tabId?: string },
    sourceWsId: string,
    sourceTabId: string,
    screenX: number,
    screenY: number,
    grabOffsetSnapshot: ReturnType<typeof getTabGrabOffset> = null,
) {
    const api = getApi();
    if (dragType === "pane" && payload.blockId) {
        // Phase 2 of SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md (issue #1077):
        // tearing a pane out spawns a floating CHILD window of the
        // source instance, NOT a new full instance. We mirror the
        // tab tear-off backend model: `TearOffBlock` creates a new
        // backend workspace+tab containing the block, and the floating
        // window's `initApp` → `initHostNewWindow` path attaches to
        // that workspace via the standard `?workspaceId=` URL param.
        // The floater renders the workspace in floating mode (no tab
        // bar / widgets / status bar — see App.tsx floatingPaneId branch).
        //
        // Tab tear-off (branch below) is unchanged — tabs continue
        // to spawn a brand-new instance with its own taskbar entry.
        const newWsId = await WorkspaceService.TearOffBlock(
            payload.blockId,
            sourceTabId,
            sourceWsId,
            true,
        );
        if (!newWsId) {
            Logger.error("dnd:cross", "TearOffBlock returned no workspace id", {
                blockId: payload.blockId,
            });
            return;
        }
        // CRITICAL: invoke the IPC FIRST, then mutate the layout on
        // success. If we delete the layout node up front and the IPC
        // fails (e.g. the H.7 mid-close gate in
        // `agentmux-cef/src/commands/floating_pane.rs` rejects with
        // "a pane is currently closing; retry shortly", or any other
        // error path), the pane would be orphaned — still in
        // `blockids` but with no layout node and no floater.
        // Reagent P1 on PR #1073.
        try {
            await invokeCommand<{ window_label: string }>("open_floating_pane_window", {
                pane_id: payload.blockId,
                workspace_id: newWsId,
                x: screenX,
                y: screenY,
                width: DEFAULT_FLOATER_WIDTH,
                height: DEFAULT_FLOATER_HEIGHT,
            });
            Logger.info("dnd:cross", "floating pane spawned", {
                blockId: payload.blockId,
                newWsId,
                screenX,
                screenY,
            });
        } catch (e) {
            Logger.error("dnd:cross", "open_floating_pane_window failed — leaving pane docked", {
                error: String(e),
                blockId: payload.blockId,
            });
            // TODO(phase-5): undo TearOffBlock here to avoid an orphaned
            // workspace if the host couldn't create the floating window.
            // For now we leave the new workspace in place; it's reachable
            // via the workspace switcher and the user can close it.
            return;
        }
        // IPC succeeded → safe to drop the docked node so the pane
        // doesn't render twice.
        const layoutModel = getLayoutModelForStaticTab();
        if (layoutModel) {
            const node = layoutModel.getNodeByBlockId(payload.blockId);
            if (node) {
                layoutModel.treeReducer({
                    type: LayoutTreeActionType.DeleteNode,
                    nodeId: node.id,
                } as LayoutTreeDeleteNodeAction);
            }
        }
    } else if (dragType === "tab" && payload.tabId) {
        const newWsId = await WorkspaceService.TearOffTab(payload.tabId, sourceWsId);
        if (newWsId) {
            // Tab anchor: convert grab-offset-in-tab to a screen point
            // so the new window's first tab lands under the cursor at
            // the same offset (no teleport on handoff).
            //
            // DPI mismatch fix (codex P2 PR #730 round 2): screenX/Y
            // here come from the host's `get_cursor_point` which uses
            // Win32 GetCursorPos = PHYSICAL pixels. The grab offset
            // was captured from `clientX` + getBoundingClientRect()
            // = CSS/DIP pixels. Subtracting raw DIP from physical px
            // would land the new window in the wrong place on any
            // display with DPR ≠ 1. Multiply offset by devicePixelRatio
            // to bring both into physical px before subtracting.
            //
            // (The tabbar.tsx::performTabTearOff path doesn't have this
            // mismatch because its screenX is `window.screenX +
            // input.clientX`, both DIP — matches the offset.)
            // Prefer the dragend-time snapshot (captured before pragmatic-dnd
            // onDrop cleared the offset). Fall back to the live store if no
            // snapshot was passed.
            const grabOffset = grabOffsetSnapshot ?? getTabGrabOffset();
            const dpr = window.devicePixelRatio || 1;
            const tabAnchorX = grabOffset ? screenX - grabOffset.x * dpr : undefined;
            const tabAnchorY = grabOffset ? screenY - grabOffset.y * dpr : undefined;
            Logger.info("dnd:cross", "performTearOff anchor", {
                screenX, screenY, dpr,
                grabOffsetX: grabOffset?.x, grabOffsetY: grabOffset?.y,
                tabAnchorX, tabAnchorY,
            });
            await openTearOffWindow(
                api,
                newWsId,
                screenX,
                screenY,
                window.outerWidth,
                window.outerHeight,
                tabAnchorX,
                tabAnchorY,
            );
        }
    }
}

export { CrossWindowDragMonitor };
