// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as MOS from "@/app/store/mos";

/** Block meta srv reads (`agent_state.rs`): this pane's launch is waiting for a sign-in. */
const LAUNCH_BLOCKED = "agent:launch_blocked";
const LAUNCH_BLOCKED_AT = "agent:launch_blocked_at_ms";

/**
 * Tell srv that this pane's launch stopped at "needs sign-in". The pane has no
 * controller then, so without this a message to its agent fails with "no
 * controller for block" instead of being held for the sign-in. Best effort: the
 * pane's own prompt does not depend on it.
 */
export async function markLaunchBlockedOnSignIn(blockId: string): Promise<void> {
    try {
        await RpcApi.SetMetaCommand(TabRpcClient, {
            oref: MOS.makeORef("block", blockId),
            meta: { [LAUNCH_BLOCKED]: "sign_in", [LAUNCH_BLOCKED_AT]: Date.now() },
        });
    } catch {
        // Best effort.
    }
}

/** The pane registered its controller: it is no longer waiting for a sign-in. Writes only when the flag is set. */
export async function clearLaunchBlocked(blockId: string): Promise<void> {
    const oref = MOS.makeORef("block", blockId);
    if (!MOS.getMuxObjectAtom<Block>(oref)()?.meta?.[LAUNCH_BLOCKED]) return;
    try {
        await RpcApi.SetMetaCommand(TabRpcClient, { oref, meta: { [LAUNCH_BLOCKED]: null, [LAUNCH_BLOCKED_AT]: null } });
    } catch {
        // Best effort: a stale flag only matters while the pane has no controller.
    }
}
