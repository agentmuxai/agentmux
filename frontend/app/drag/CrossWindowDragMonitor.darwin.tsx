// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CrossWindowDragMonitor — macOS
 *
 * On macOS, WebKit reliably delivers `dragend` to the source element even when
 * the drop occurs over a native app. No OLE fallback needed.
 */

import { atoms, getApi } from "@/store/global";
import { sleep } from "@/util/util";
import { invokeCommand } from "@/app/platform/ipc";
import { WorkspaceService } from "@/app/store/services";
import { getLayoutModelForStaticTab, LayoutTreeActionType, LayoutTreeDeleteNodeAction } from "@/layout/index";
import { Logger } from "@/util/logger";
import { openTearOffWindow, measureSourcePaneSize } from "./tear-off-pool-helper";
import { onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import type { LayoutNode } from "@/layout/lib/types";
import { dragEscaped, setDragEscaped } from "@/app/tab/tabbar-dnd";

export type DragItemPayload =
    // sourceTabId: the tab the tile drag originated in — consumed by the
    // in-window pane->tab drop path (droppable-tab.tsx) to build the
    // cross-tab redock call. Optional: cross-window consumers don't use it.
    | { kind: "tile"; node: LayoutNode; sourceTabId?: string }
    | { kind: "tab"; tabId: string; workspaceId: string };

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
        Logger.debug("dnd:cross", "CrossWindowDragMonitor mounted (darwin)", { windowLabel: windowLabelRef });

        const handleDragEnd = async (e: DragEvent) => {
            const payload = _currentDragPayload;
            _currentDragPayload = null;

            Logger.info("dnd:cross", "dragend fired", { hasPayload: !!payload, dropEffect: e.dataTransfer?.dropEffect });

            if (!payload) return;

            // Escape was pressed during this drag (see the CGEventTap-driven
            // tabdrag:escape-pressed listener in tab-tearoff-events.ts, or the
            // DOM keydown fallback in tab-reorder.ts) — abort here too. This
            // handler is the genuine cross-window tear-off/merge commit path
            // (reached when the drop lands outside this window's own tab
            // strip's pragmatic-dnd monitor entirely, e.g. onto another
            // AgentMux window or the desktop) — reagent PR #2310 P1: without
            // this check, an escaped drag could still spawn a tear-off window
            // or merge via this path even though tab-reorder.ts's onDrop
            // correctly no-ops for the plain in-window case.
            if (dragEscaped) {
                setDragEscaped(false);
                Logger.info("dnd:cross", "cross-window drag aborted via Escape");
                return;
            }

            // Drop coordinates straight from the DOM event. `screenX/Y` are
            // top-left-origin screen coords in CSS px (= DIP), which is exactly
            // what CEF Views window positioning expects on macOS — no host
            // `get_cursor_point` round-trip needed (that command is a Windows-
            // only `GetCursorPos`; its macOS stub returns 0,0, which would open
            // the floater in the screen corner).
            const dropX = e.screenX;
            const dropY = e.screenY;

            await sleep(50);
            await handleCrossWindowDragEnd(payload, windowLabelRef, dropX, dropY);
        };

        // Suppress the "operation not allowed" cursor during a cross-window
        // drag. WebKit (and all HTML5 DnD implementations) shows the
        // prohibited-circle cursor whenever the cursor is over a region with
        // no `dragover` preventDefault — i.e., everywhere outside our own
        // tab bar and the workspace surface. The JS-driven tear-off on
        // `dragend` then completes successfully, so the user sees a clear
        // mismatch: cursor says "no", behavior says "yes". Telling the
        // browser "this is a valid drop target everywhere" while a cross-
        // window drag is in flight (i.e. _currentDragPayload is set) keeps
        // the cursor on the "move" affordance throughout, matching the
        // actual outcome. Gated on payload so we don't hijack unrelated DnD
        // sessions (file drops, etc.).
        const handleDragOver = (e: DragEvent) => {
            if (_currentDragPayload == null) return;
            e.preventDefault();
            if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
        };

        document.addEventListener("dragover", handleDragOver);
        document.addEventListener("dragend", handleDragEnd);
        onCleanup(() => {
            document.removeEventListener("dragover", handleDragOver);
            document.removeEventListener("dragend", handleDragEnd);
        });
    });

    return null;
}

async function handleCrossWindowDragEnd(
    payload: DragItemPayload,
    sourceWindow: string | null,
    dropX: number,
    dropY: number,
) {
    // `get_cursor_point` is a Windows-only host command (GetCursorPos); on
    // macOS it returns 0,0. Use the DOM drop coordinates (top-left origin,
    // CSS px = DIP) instead — correct for CEF Views positioning and for the
    // cross-window hit-test below.
    const cursorPoint = { x: dropX, y: dropY };

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

    // Hoisted so the catch can release the session even when startCrossDrag
    // succeeded but a later step (TearOffBlock / drop) threw.
    let dragId: string | null = null;
    try {
        dragId = await api.startCrossDrag(dragType, src, workspace.oid, activeTabId, dragPayloadForApi);
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
        // CRITICAL: release the host drag session so a failed drop/tear-off
        // can't jam every future tear-off with "drag session already active".
        if (dragId) { try { await api.cancelCrossDrag(dragId); } catch {} }
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
        // PANE → chromeless floating window (just the pane: no tab bar, no
        // widget bar). Mirrors the Windows pane branch
        // (CrossWindowDragMonitor.win32.tsx). `TearOffBlock` moves the block
        // into a fresh backend workspace+tab; the floating window's
        // `initApp` → `initHostNewWindow` path attaches to it via
        // `?workspaceId=`, and the `?floatingPaneId=` URL param makes the
        // frontend render `<FloatingPaneWorkspace>` (chromeless) instead of
        // `<Workspace>`. See SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md.

        // Snapshot the source pane's rendered size BEFORE TearOffBlock —
        // that mutation unmounts the source DOM element.
        const { width: floaterWidth, height: floaterHeight } = measureSourcePaneSize(
            payload.blockId,
        );

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

        // CRITICAL: invoke the IPC FIRST, then mutate the layout on success.
        // If we deleted the layout node up front and the IPC failed (e.g. the
        // H.7 mid-close gate rejects), the pane would be orphaned — still in
        // `blockids` but with no layout node and no floater. Reagent P1 on
        // PR #1073 (Windows path).
        try {
            await invokeCommand<{ window_label: string }>("open_floating_pane_window", {
                pane_id: payload.blockId,
                workspace_id: newWsId,
                x: screenX,
                y: screenY,
                width: floaterWidth,
                height: floaterHeight,
            });
            Logger.info("dnd:cross", "floating pane spawned (darwin)", {
                blockId: payload.blockId,
                newWsId,
                screenX,
                screenY,
            });
        } catch (e) {
            const msg = String(e);
            if (msg.includes("currently closing")) {
                await sleep(350);
                try {
                    await invokeCommand<{ window_label: string }>("open_floating_pane_window", {
                        pane_id: payload.blockId,
                        workspace_id: newWsId,
                        x: screenX,
                        y: screenY,
                        width: floaterWidth,
                        height: floaterHeight,
                    });
                } catch (e2) {
                    Logger.error("dnd:cross", "open_floating_pane_window failed after retry", {
                        error: String(e2),
                        blockId: payload.blockId,
                    });
                    return;
                }
            } else {
                Logger.error("dnd:cross", "open_floating_pane_window failed — leaving pane docked", {
                    error: msg,
                    blockId: payload.blockId,
                });
                return;
            }
        }

        // IPC succeeded → drop the docked node so the pane doesn't render twice.
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
        // TAB → full workspace window (unchanged): tab bar + widgets.
        const newWsId = await WorkspaceService.TearOffTab(payload.tabId, sourceWsId);
        if (newWsId) await openTearOffWindow(api, newWsId, screenX, screenY);
    }
}

export { CrossWindowDragMonitor };
