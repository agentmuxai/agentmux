// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A hidden History tab does no work (spec §6.9), and a stale reader who scrolls back to the bottom gets
 * the newest page. Split out of AgentHistoryView.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createEffect, on } from "solid-js";
import { GAP_FILL_MAX_LINES } from "../transcript-cursor";
import type { HistoryReader } from "./history-reader-state";

export function useHistoryDormancy(s: HistoryReader): void {
    // ---- Dormancy: a hidden tab does no work (spec §6.9) ----
    // Events are not handed to the cursor while hidden — nothing is decoded
    // or parsed; the tab only notes one arrived. On reveal, one line-count
    // read catches up: the cursor fills a gap it can fill by range reads,
    // and a larger one (or a different stream/generation) reloads.
    // (`missedWhileHidden` and `resetWhileHidden` live on the reader: a truncate/replace/delete
    // arrived while hidden, and only a reload can recover it: a count read of the new generation
    // is ignored by the cursor, which is pinned to the old one.)
    const catchUp = async (): Promise<void> => {
        const c = s.cursor;
        const pin = c?.position();
        if (!c || !pin) return s.act.recover();
        try {
            const resp = await RpcApi.BlockfileLineCountCommand(
                TabRpcClient,
                { block_id: s.readBlockId, filename: "output" },
                { timeout: 5_000 }
            );
            if (s.disposed || c !== s.cursor) return;
            if (!resp?.stream || !resp.gen || resp.stream !== pin.stream || resp.gen !== pin.gen) return s.act.recover();
            if (resp.count - pin.next > GAP_FILL_MAX_LINES) return s.act.recover();
            c.observeCount(resp.count, resp.stream, resp.gen);
        } catch {
            if (!s.disposed) s.act.recover();
        }
    };
    createEffect(
        on(s.hidden, (isHidden) => {
            if (isHidden) {
                // An armed publish would render a hidden tab: hold it.
                if (s.publishTimer !== undefined) {
                    clearTimeout(s.publishTimer);
                    s.publishTimer = undefined;
                    s.dirty = true;
                }
                return;
            }
            if (s.dirty) s.act.publishSoon();
            if (s.resetWhileHidden) {
                s.resetWhileHidden = false;
                s.missedWhileHidden = false;
                s.act.recover();
            } else if (s.missedWhileHidden) {
                s.missedWhileHidden = false;
                void catchUp();
            }
        })
    );

    // A stale reader who scrolls back to the bottom gets the newest page.
    createEffect(() => {
        if (s.stale() && s.followingSignal() && s.mode() === "live") void s.act.load();
    });
}
