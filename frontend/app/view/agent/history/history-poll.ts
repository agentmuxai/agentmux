// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Lines no event brings: another block or srv instance appending to the agent's shared zone (same poll as the live pane's). */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { TranscriptCursor } from "../transcript-cursor";

/** Line-count poll cadence — same as the live pane. */
const SHARED_ZONE_POLL_MS = 5_000;

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
    let polling = false;
    const timer = setInterval(async () => {
        reloadIfSkipped();
        const pin = cursor()?.position();
        if (!pin || !pin.stream.startsWith("g:") || polling || hidden() || document.hidden) return;
        polling = true;
        try {
            const resp = await RpcApi.BlockfileLineCountCommand(
                TabRpcClient,
                { block_id: readBlockId, filename: "output" },
                { timeout: 5_000 }
            );
            if (!resp?.stream || !resp.gen) return;
            if (resp.stream !== pin.stream || resp.gen !== pin.gen) {
                // Recreated under us (see appendRecords): start over.
                lastCount = null;
                recover();
                return;
            }
            if (lastCount && lastCount.stream === resp.stream && lastCount.gen === resp.gen) {
                cursor()?.observeCount(lastCount.count, resp.stream, resp.gen);
            }
            lastCount = { count: resp.count, stream: resp.stream, gen: resp.gen };
        } catch {
            // Soft: the next tick or event catches up.
        } finally {
            polling = false;
        }
    }, SHARED_ZONE_POLL_MS);
    return () => clearInterval(timer);
}
