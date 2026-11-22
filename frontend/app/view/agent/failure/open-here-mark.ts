// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tell srv that a pane was bound with "Open here" (`agent:open_here`, read in `open_here_mark.rs`).
 *
 * srv refuses the second pane of an agent that is open in another pane, and the Open here pane answers
 * that refusal at once with Move here (failure/open-here.ts). With the mark srv leaves that one refusal
 * out of the pane's history, where it would stay as a red line above the conversation, and still shows
 * it on the pane's recovery row. srv clears the mark when the agent has moved; a mark that was never
 * answered (the agent was free) is cleared here after the same time the window waits for the refusal.
 * Best effort: without it the pane works, and keeps the line.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as MOS from "@/app/store/mos";
import { OPEN_HERE_MARK_MS } from "./open-here";

const OPEN_HERE_META = "agent:open_here";

async function setMark(blockId: string, on: boolean): Promise<void> {
    const oref = MOS.makeORef("block", blockId);
    if (!on && !MOS.getMuxObjectAtom<Block>(oref)()?.meta?.[OPEN_HERE_META]) return;
    try {
        await RpcApi.SetMetaCommand(TabRpcClient, { oref, meta: { [OPEN_HERE_META]: on ? true : null } });
    } catch {
        // Best effort.
    }
}

/** Mark the pane before it is bound, so the mark is there when the refusal is. */
export async function markOpenHerePane(blockId: string): Promise<void> {
    await setMark(blockId, true);
    setTimeout(() => void setMark(blockId, false), OPEN_HERE_MARK_MS);
}

/** Drop the mark of a pane whose open failed. */
export function unmarkOpenHerePane(blockId: string): Promise<void> {
    return setMark(blockId, false);
}
