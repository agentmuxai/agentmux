// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Dragging ONE Pane Tab out of the window turns it into a floating pane.
 * Shared by the three per-platform `CrossWindowDragMonitor`s (their
 * `"pane-tab"` payload branch), so the rules live in one place.
 *
 * Reuses the whole-pane tear-off's building blocks unchanged:
 * `TearOffBlock` moves the SAME block into a new workspace+tab (a live agent
 * session, terminal or editor buffer survives), and
 * `open_floating_pane_window` shows it. What's specific to a single tab:
 * - an in-window drop never tears off (macOS/Linux have no host hit-test);
 * - the source pane keeps its other tabs (stack-safe removal);
 * - the window only shrinks when the pane itself leaves;
 * - a failed window open is rolled back, and the tab goes back where it was.
 *
 * SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.5.
 */

import { WorkspaceService } from "@/app/store/services";
import { removeMovedBlock } from "@/layout/lib/layoutMagnify";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import { findNodeByBlockId } from "@/layout/lib/layoutNode";
import { effectiveStack } from "@/layout/lib/stackMembers";
import { atoms, getApi } from "@/store/global";
import { Logger } from "@/util/logger";
import { sleep } from "@/util/util";
import { floaterSizeFromRect, measureMotherResize, measureSourcePaneSize } from "./tear-off-pool-helper";

/** The cross-window drag payload a Pane Tab pill sets on drag start
 *  (PaneTabStrip). `paneSize` is its pane's rect (CSS/DIP px) at that moment:
 *  the floater opens at the size of the pane the tab came from. */
export interface PaneTabDragPayload {
    kind: "pane-tab";
    blockId: string;
    sourceNodeId: string;
    sourceTabId: string;
    paneSize?: { width: number; height: number };
}

export type Platform = "win32" | "darwin" | "linux";

/** Whether a screen point (CSS/DIP px) lies inside this window's own frame.
 *  macOS/Linux have no host window hit-test (it's a stub that always says
 *  "no window"), so without this a Pane Tab dropped on dead space INSIDE the
 *  window, where no drop target claims it, would tear off. */
export function isInsideWindow(
    x: number,
    y: number,
    win: Pick<Window, "screenX" | "screenY" | "outerWidth" | "outerHeight"> = window
): boolean {
    return x >= win.screenX && x < win.screenX + win.outerWidth && y >= win.screenY && y < win.screenY + win.outerHeight;
}

/** What `tearOffPaneTab` did. */
export type PaneTabTearOffResult = "torn-off" | "rolled-back" | "failed" | "skipped";

/**
 * The monitors' entry point for a `"pane-tab"` payload that ended its drag.
 * `point` is the drop point in the units the host expects for this platform
 * (Windows: physical px from `get_cursor_point`; macOS/Linux: DIP from the
 * `dragend` event). Tears off only when the drop landed on NO AgentMux
 * window. A drop on this window (no target claimed it) or on another
 * AgentMux window (not supported for a single tab yet) is a cancel.
 */
export async function handlePaneTabDragEnd(
    payload: PaneTabDragPayload,
    sourceWindow: string | null,
    point: { x: number; y: number },
    platform: Platform
): Promise<void> {
    if (platform !== "win32" && isInsideWindow(point.x, point.y)) {
        Logger.info("dnd:cross", "pane-tab drop inside the window with no target — cancel", { blockId: payload.blockId });
        return;
    }
    const api = getApi();
    const workspace = atoms.workspace() as Workspace | undefined;
    if (!workspace) {
        Logger.warn("dnd:cross", "no workspace found — aborting pane-tab drag");
        return;
    }
    const src = sourceWindow ?? "main";
    let dragId: string | null = null;
    try {
        dragId = await api.startCrossDrag("pane", src, workspace.oid, payload.sourceTabId, { blockId: payload.blockId });
        const targetWindow = await api.updateCrossDrag(dragId, point.x, point.y);
        if (targetWindow) {
            Logger.info("dnd:cross", "pane-tab dropped on an AgentMux window — cancel", {
                blockId: payload.blockId,
                targetWindow,
                sameWindow: targetWindow === src,
            });
            await api.cancelCrossDrag(dragId);
            return;
        }
        await tearOffPaneTab(payload, {
            screenX: point.x,
            screenY: point.y,
            sourceWorkspaceId: workspace.oid,
            sourceWindowLabel: sourceWindow,
            platform,
        });
        await api.completeCrossDrag(dragId, null, point.x, point.y);
        if (platform === "win32") {
            try {
                await api.releaseDragCapture();
            } catch {
                // best-effort, same as the whole-pane path
            }
        }
    } catch (e) {
        Logger.error("dnd:cross", "pane-tab drag error", { error: String(e), blockId: payload.blockId });
        // Release the host's singleton drag session, or every later tear-off
        // is rejected with "drag session already active".
        if (dragId) {
            try {
                await api.cancelCrossDrag(dragId);
            } catch {
                // already gone
            }
        }
    }
}

/**
 * Tear one Pane Tab off into a floating pane window. See this file's header
 * for the rules. `screenX/Y` are in the host's units for `platform`.
 */
export async function tearOffPaneTab(
    payload: PaneTabDragPayload,
    opts: {
        screenX: number;
        screenY: number;
        sourceWorkspaceId: string;
        sourceWindowLabel: string | null;
        platform: Platform;
    }
): Promise<PaneTabTearOffResult> {
    const { blockId, sourceTabId } = payload;
    const model = getLayoutModelForTabById(sourceTabId);
    const root = model?.treeState.rootNode;
    const leaf = root && findNodeByBlockId(root, blockId);
    if (!model || !leaf?.data) {
        Logger.warn("dnd:cross", "pane-tab tear-off: tab is no longer in its pane — skipped", { blockId });
        return "skipped";
    }
    const members = effectiveStack(leaf.data);
    const wasOnlyTab = members.length <= 1;
    // Any tab that stays behind in the pane — the rollback target.
    const survivor = members.find((m) => m !== blockId);

    // Measure BEFORE TearOffBlock: the source DOM goes away with it.
    const { width, height } = payload.paneSize ? floaterSizeFromRect(payload.paneSize) : measureSourcePaneSize(blockId);
    // The window shrinks by the pane's width only when the PANE leaves (the
    // tab was its only one) — never for a pane that keeps other tabs. macOS
    // doesn't resize the mother window at all (same as its whole-pane path).
    const motherResizeToWidth =
        opts.platform !== "darwin" && wasOnlyTab && !model.treeState.magnifiedNodeId
            ? measureMotherResize(blockId)
            : undefined;

    const newWsId = await WorkspaceService.TearOffBlock(blockId, sourceTabId, opts.sourceWorkspaceId, true);
    if (!newWsId) {
        Logger.error("dnd:cross", "pane-tab tear-off: TearOffBlock returned no workspace id", { blockId });
        return "failed";
    }

    const opened = await openFloatingPaneWindow({
        pane_id: blockId,
        workspace_id: newWsId,
        x: opts.screenX,
        y: opts.screenY,
        width,
        height,
        ...(opts.platform === "darwin"
            ? {}
            : { source_window_label: opts.sourceWindowLabel, mother_resize_to_width: motherResizeToWidth }),
    });
    if (opened) {
        // The floater shows it now: drop it from the source pane, stack-safe.
        // A no-op if the backend's queued `delete` for this move already did.
        removeMovedBlock(model, blockId);
        Logger.info("dnd:cross", "pane-tab torn off into a floating pane", { blockId, newWsId, wasOnlyTab });
        return "torn-off";
    }

    return (await rollBackTearOff(blockId, newWsId, sourceTabId, opts.sourceWorkspaceId, survivor))
        ? "rolled-back"
        : "failed";
}

/** `open_floating_pane_window`, with the same one-shot retry the whole-pane
 *  path uses for the brief "a pane is currently closing" window. */
async function openFloatingPaneWindow(args: Record<string, unknown>): Promise<boolean> {
    try {
        await getApi().windows.openFloatingPane(args);
        return true;
    } catch (e) {
        if (!String(e).includes("currently closing")) {
            Logger.error("dnd:cross", "open_floating_pane_window failed", { error: String(e), paneId: args.pane_id });
            return false;
        }
    }
    await sleep(350);
    try {
        await getApi().windows.openFloatingPane(args);
        return true;
    } catch (e) {
        Logger.error("dnd:cross", "open_floating_pane_window failed after retry", { error: String(e), paneId: args.pane_id });
        return false;
    }
}

/**
 * Undo a tear-off whose window never opened: move the block straight back
 * (the same identity-preserving `RedockFloatingPane` a floater's own redock
 * uses), then delete the now-empty workspace so nothing is left dangling.
 * - The pane still has tabs → back as a TAB of it (`asTab`), active.
 * - The pane went with it → back as its own pane (plain insert). Its old
 *   position isn't recoverable, but nothing is lost.
 * If the source window tab was auto-closed (it held only this block), there's
 * nowhere to put it back; the block stays in the new workspace, reachable
 * from the workspace switcher, and this is logged.
 */
async function rollBackTearOff(
    blockId: string,
    newWsId: string,
    sourceTabId: string,
    sourceWsId: string,
    survivor: string | undefined
): Promise<boolean> {
    try {
        const newWs = await WorkspaceService.GetWorkspace(newWsId);
        const floaterTabId = newWs?.tabids?.[0];
        if (!floaterTabId) throw new Error("the new workspace has no tab");
        if (survivor) {
            await WorkspaceService.RedockFloatingPane(blockId, floaterTabId, newWsId, sourceTabId, sourceWsId, survivor, null, true);
        } else {
            await WorkspaceService.RedockFloatingPane(blockId, floaterTabId, newWsId, sourceTabId, sourceWsId);
        }
        await WorkspaceService.DeleteWorkspace(newWsId);
        Logger.warn("dnd:cross", "pane-tab tear-off rolled back: the floating window didn't open", {
            blockId,
            asTab: !!survivor,
        });
        return true;
    } catch (e) {
        Logger.error("dnd:cross", "pane-tab tear-off rollback failed — the tab is in its own workspace", {
            error: String(e),
            blockId,
            newWsId,
        });
        return false;
    }
}
