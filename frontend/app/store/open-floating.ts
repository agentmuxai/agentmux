// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Open a pane as a floating window, the way the widget bar's "Open in Floating Pane" does (srv creates the
 * block and tears it into its own window). Loaded on the first float, so the plain open path never pulls
 * the RPC client into the modules it is called from.
 */

import { atoms } from "./global";
import { RpcApi } from "./rpc-api";
import { TabRpcClient } from "./rpc-util";

/** A new floating pane for `blockDef`; null for a block that can't be opened that way (srv builds the
 *  block from its meta alone, so one that brings files is opened docked as usual). */
export async function openBlockFloating(blockDef: BlockDef): Promise<string | null> {
    const meta = blockDef.meta as Record<string, unknown> | undefined;
    const view = meta?.["view"];
    if (typeof view !== "string" || !view || (blockDef.files && Object.keys(blockDef.files).length > 0)) return null;
    const opened = await RpcApi.PaneOpenCommand(
        TabRpcClient,
        // This window's tab: with none srv picks the first workspace with an active tab, which can be another window's.
        { view, meta: blockDef.meta, floating: true, tab_id: atoms.activeTabId() },
        {}
    );
    return opened?.block_id ?? null;
}
