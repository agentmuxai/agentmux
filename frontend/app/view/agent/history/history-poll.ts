// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Lines no event brings: another block or srv instance appending to the agent's shared zone (same poll as the live pane's). */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { SHARED_ZONE_POLL_MS, startCadencedPoll, type PollOutcome } from "../shared-zone-cadence";
import type { TranscriptCursor } from "../transcript-cursor";

/** Only while shown; a hidden reader catches up from the first poll after reveal. Returns the stop. */
export function startHistoryPoll(p: {
    readBlockId: string;
    /** Read at each use: a reload swaps the cursor under a poll that is waiting on a reply. */
    cursor: () => TranscriptCursor | null;
    hidden: () => boolean;
    reloadIfSkipped: () => void;
    recover: () => void;
}): () => void {
    const { readBlockId, cursor, hidden, reloadIfSkipped, recover } = p;
    let lastCount: { count: number; stream: string; gen: string } | null = null;
    // Local and cheap, so it keeps the steady pace whatever the count does.
    const skipCheck = setInterval(reloadIfSkipped, SHARED_ZONE_POLL_MS);
    const stopPoll = startCadencedPoll(async (): Promise<PollOutcome> => {
        const pin = cursor()?.position();
        if (!pin || !pin.stream.startsWith("g:") || hidden() || document.hidden) return "skipped";
        const resp = await RpcApi.BlockfileLineCountCommand(
            TabRpcClient,
            { block_id: readBlockId, filename: "output" },
            { timeout: 5_000 }
        );
        if (!resp?.stream || !resp.gen) return "skipped";
        if (resp.stream !== pin.stream || resp.gen !== pin.gen) {
            // Recreated under us (see appendRecords): start over.
            lastCount = null;
            recover();
            return "moved";
        }
        const same = lastCount !== null && lastCount.stream === resp.stream && lastCount.gen === resp.gen;
        if (same) cursor()?.observeCount(lastCount!.count, resp.stream, resp.gen);
        const still = same && lastCount!.count === resp.count;
        lastCount = { count: resp.count, stream: resp.stream, gen: resp.gen };
        return still ? "still" : "moved";
    });
    return () => {
        clearInterval(skipCheck);
        stopPoll();
    };
}
