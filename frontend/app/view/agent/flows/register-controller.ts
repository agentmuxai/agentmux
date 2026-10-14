// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { BlockService } from "@/app/store/services";
import { staticTabId } from "@/app/store/global";
import type { BlockControllerRuntimeStatus } from "@/types/rpc/BlockControllerRuntimeStatus";

/** Register this pane's controller (or, with `forceRestart`, kill and
 *  respawn it onto a new credential), then read its runtime status. Shared
 *  by the launch flow's Phase 3 and the pane's post-login restart.
 *  `termSize` seeds the PTY so the CLI wraps at the pane's width from its
 *  first byte (AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md). Throws what the
 *  RPCs throw. */
export async function registerController(
    blockId: string,
    opts: { forceRestart: boolean; termSize?: { rows: number; cols: number } },
): Promise<BlockControllerRuntimeStatus | null> {
    await RpcApi.ControllerResyncCommand(TabRpcClient, {
        tabid: staticTabId(),
        blockid: blockId,
        forcerestart: opts.forceRestart,
        ...(opts.termSize ? { rtopts: { termsize: opts.termSize } } : {}),
    });
    return (await BlockService.GetControllerStatus(blockId)) ?? null;
}
