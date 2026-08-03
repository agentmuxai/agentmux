// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CrossWindowDragMonitor — Linux
 *
 * On Linux, WebKitGTK reliably delivers `dragend` to the source element even when
 * the drop occurs over a native app. No OLE fallback needed.
 */

import { atoms, getApi } from "@/store/global";
import { sleep } from "@/util/util";
import { WorkspaceService } from "@/app/store/services";
import { Logger } from "@/util/logger";
import { openTearOffWindow, measureSourcePaneSize, measureMotherResize } from "./tear-off-pool-helper";
import { getTabGrabOffset } from "@/app/tab/tab-grab-offset";
import { invokeCommand } from "@/app/platform/ipc";
import { onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import { getLayoutModelForStaticTab, LayoutTreeActionType, LayoutTreeDeleteNodeAction } from "@/layout/index";
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
        Logger.debug("dnd:cross", "CrossWindowDragMonitor mounted (linux)", { windowLabel: windowLabelRef });

        const handleDragEnd = async (e: DragEvent) => {
            const payload = _currentDragPayload;
            _currentDragPayload = null;

            Logger.info("dnd:cross", "dragend fired", { hasPayload: !!payload, dropEffect: e.dataTransfer?.dropEffect });

            if (!payload) return;

            // Escape was pressed during this drag — abort here too, same
            // reasoning as the darwin variant (reagent PR #2310 P1). Linux
            // doesn't have a native-hook-driven escape signal yet (that's
            // macOS-only so far), but the DOM keydown fallback in
            // tab-reorder.ts still sets this flag, so honoring it here costs
            // nothing and closes the same gap if/when it applies.
            if (dragEscaped) {
                setDragEscaped(false);
                Logger.info("dnd:cross", "cross-window drag aborted via Escape");
                return;
            }

            // Drop coordinates straight from the DOM event. `screenX/Y` are
            // top-left-origin screen coords in CSS px (= DIP), which is what
            // CEF Views window positioning expects on Linux. Don't round-trip
            // through `get_cursor_point` — that command is Windows-only
            // (returns 0,0 on non-Windows builds, drag.rs:211-212), which
            // would open the floater at the screen corner.
            const dropX = e.screenX;
            const dropY = e.screenY;

            await sleep(50);
            await handleCrossWindowDragEnd(payload, windowLabelRef, dropX, dropY);
        };

        // Suppress the "operation not allowed" cursor during a cross-window
        // drag. WebKitGTK (and all HTML5 DnD implementations) shows the
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
    // Linux it returns 0,0. Use the DOM drop coordinates (top-left origin,
    // CSS px = DIP) — correct for CEF Views positioning.
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
            await performTearOff(dragType, dragPayloadForApi, workspace.oid, activeTabId, cursorPoint.x, cursorPoint.y, sourceWindow);
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
    screenY: number,
    sourceWindowLabel: string | null = null,
) {
    const api = getApi();
    if (dragType === "pane" && payload.blockId) {
        // PANE → chromeless floating window (just the pane: no tab bar, no
        // widget bar). Mirrors the Windows and macOS pane branches.
        // `TearOffBlock` moves the block into a fresh backend workspace+tab;
        // the floating window's `initApp` → `initHostNewWindow` path
        // attaches to it via `?workspaceId=`, and the `?floatingPaneId=`
        // URL param makes the frontend render `<FloatingPaneWorkspace>`
        // (chromeless) instead of `<Workspace>`.
        //
        // Backend creates the frameless top-level via
        // `post_create_window(frameless=true)` — the SAME CEF Views path
        // the legacy tear-off used — so on Linux this is purely a
        // frontend routing change. #1182 widened the backend's
        // `open_floating_pane_window` non-Windows branch from "not
        // implemented" to a real impl that runs identically on Linux
        // and macOS.
        //
        // See docs/specs/SPEC_LINUX_FLOATING_PANE_TEAROFF_2026_05_30.md
        // (mirrors SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md §"Phase A").

        // Snapshot the source pane's rendered size BEFORE TearOffBlock —
        // that mutation removes the block from the source layout and
        // unmounts the source DOM element.
        const { width: floaterWidth, height: floaterHeight } = measureSourcePaneSize(
            payload.blockId,
        );
        // Compute mother window resize: if the pane spans the full height of
        // the layout container (top-to-bottom column), the mother shrinks by
        // the pane's width so remaining panes keep their sizes unchanged.
        // Skip when a pane is magnified — layout container dimensions are
        // misleading in that mode (spec §9).
        // SPEC: SPEC_PANE_TEAROFF_MOTHER_RESIZE_2026_06_20.md
        const layoutModelForResize = getLayoutModelForStaticTab();
        const motherResizeToWidth = layoutModelForResize?.treeState?.magnifiedNodeId
            ? undefined
            : measureMotherResize(payload.blockId);

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
        // success. If we deleted the layout node up front and the IPC
        // failed (e.g. the H.7 mid-close gate rejects), the pane would
        // be orphaned — still in `blockids` but with no layout node and
        // no floater. Reagent P1 on PR #1073 (Windows path); the same
        // ordering is preserved here.
        try {
            await invokeCommand<{ window_label: string }>("open_floating_pane_window", {
                pane_id: payload.blockId,
                workspace_id: newWsId,
                x: screenX,
                y: screenY,
                width: floaterWidth,
                height: floaterHeight,
                source_window_label: sourceWindowLabel,
                mother_resize_to_width: motherResizeToWidth,
            });
            Logger.info("dnd:cross", "floating pane spawned (linux)", {
                blockId: payload.blockId,
                newWsId,
                screenX,
                screenY,
                width: floaterWidth,
                height: floaterHeight,
                sourceWindowLabel,
                motherResizeToWidth,
            });
        } catch (err) {
            const msg = String(err);
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
                        source_window_label: sourceWindowLabel,
                        mother_resize_to_width: motherResizeToWidth,
                    });
                } catch (e2) {
                    Logger.error("dnd:cross", "open_floating_pane_window failed after retry", {
                        error: String(e2),
                        blockId: payload.blockId,
                        newWsId,
                    });
                    return;
                }
            } else {
                Logger.error("dnd:cross", "open_floating_pane_window failed — leaving pane docked", {
                    error: msg,
                    blockId: payload.blockId,
                    newWsId,
                });
                return;
            }
        }

        // IPC succeeded → drop the docked layout node so the pane
        // doesn't render twice (once in the source tab, once in the
        // floater). Mirrors the .win32 and .darwin siblings.
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
            // Tab anchor in DIP — screenX/Y are DOM `e.screenX/Y` (CSS px =
            // DIP), and `getTabGrabOffset()` is also DOM client-space (DIP).
            // Linux CEF Views positions in DIP, so no DPR scale (the win32
            // sibling needs `* dpr` because `get_cursor_point` there returns
            // physical px from `GetCursorPos`).
            const grabOffset = getTabGrabOffset();
            const tabAnchorX = grabOffset ? screenX - grabOffset.x : undefined;
            const tabAnchorY = grabOffset ? screenY - grabOffset.y : undefined;
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
