// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Navigate to the pane for a given block ID, switching tabs and windows as
 * needed. Tries the current window first (fast path), then searches all
 * other workspaces and brings the containing window to focus.
 *
 * Extracted from swarm-view.tsx (its original, still-only-real
 * implementation) so other status-bar/breakdown-style views — e.g.
 * TokenBreakdownPopover's per-agent rows, see
 * SPEC_STATUSBAR_TOKEN_PANEL_BY_AGENT_2026_08_30.md — can jump to an
 * agent's pane the same way swarm's own rows already do, without
 * duplicating the cross-window search.
 */

import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WorkspaceService } from "@/app/store/services";
import { WOS, workspace, setActiveTab, getApi } from "@/app/store/global";

export async function focusBlock(blockId: string): Promise<void> {
    // Fast path: search the current window's workspace.
    const ws = workspace();
    if (ws) {
        const allTabIds = [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
        for (const tabId of allTabIds) {
            const layoutModel = getLayoutModelForTabById(tabId);
            const node = layoutModel?.getNodeByBlockId(blockId);
            if (node?.id != null) {
                await setActiveTab(tabId);
                layoutModel.focusNode(node.id);
                return;
            }
        }
    }
    // Slow path: agent is in a different window — query all workspaces.
    // We need Tab.blockids to find which tab holds the block. Layout models for
    // other-window tabs are not available from this renderer. We fetch each Tab
    // from the server (cache hit is instant; miss triggers one server round-trip).
    // focusNode is intentionally omitted: a layout model in another renderer
    // process cannot be driven from this side.
    const allWorkspaces = await RpcApi.WorkspaceListCommand(TabRpcClient);
    for (const wsInfo of allWorkspaces) {
        if (wsInfo.workspacedata.oid === ws?.oid) continue;
        const wsData = wsInfo.workspacedata;
        const allTabIds = [...(wsData.pinnedtabids ?? []), ...(wsData.tabids ?? [])];
        for (const tabId of allTabIds) {
            const oref = WOS.makeORef("tab", tabId);
            // Use cached value when available; reloadWaveObject on cache miss.
            const cached = WOS.getObjectValue<Tab>(oref);
            const tab = cached ?? (await WOS.reloadWaveObject<Tab>(oref));
            if (!tab?.blockids?.includes(blockId)) continue;
            await WorkspaceService.SetActiveTab(wsData.oid, tabId);
            const instances = await getApi().listWindowInstances();
            const instance = instances.find((i) => i.windowId === wsInfo.windowid);
            if (instance?.label) {
                await getApi().focusWindow(instance.label);
            }
            return;
        }
    }
}
