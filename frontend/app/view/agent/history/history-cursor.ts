// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The reader's TranscriptCursor: records after the loaded page are read by range from the agent's output file. */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { TranscriptCursor } from "../transcript-cursor";

export function createHistoryCursor(p: {
    readBlockId: string;
    /** Records land here in order, exactly once. */
    deliver: (text: string) => void;
    /** The stream was truncated, replaced or deleted: what the reader holds no longer describes it. Start over. */
    reset: () => void;
    /** The cursor warned (it skipped a gap, say): the caller checks what it skipped. */
    onSkipWarning: () => void;
}): TranscriptCursor {
    const { readBlockId } = p;
    return new TranscriptCursor({
        deliver: p.deliver,
        echo: p.deliver,
        isOwnEcho: () => false,
        reset: p.reset,
        readRange: async (offset, limit, expectGen) => {
            const resp = await RpcApi.BlockfileReadRangeCommand(
                TabRpcClient,
                { block_id: readBlockId, filename: "output", offset, limit, expect_gen: expectGen },
                { timeout: 15_000 }
            );
            return {
                lines: resp?.lines ?? [],
                stream: resp?.stream,
                gen: resp?.gen,
                genMismatch: resp?.gen_mismatch,
            };
        },
        log: (message, level) => {
            if (level === "warn") {
                console.warn(`[AgentHistoryView] ${message}`);
                p.onSkipWarning();
            } else {
                console.debug(`[AgentHistoryView] ${message}`);
            }
        },
    });
}
