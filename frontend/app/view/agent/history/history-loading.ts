// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The History reader's loads: the newest page followed from where it ends, and a static page for a jump
 * or an archive. Split out of AgentHistoryView.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { batch } from "solid-js";
import { historyPin } from "../transcript-cursor";
import { HISTORY_PAGE, type HistoryReader } from "./history-reader-state";

export function useHistoryLoading(s: HistoryReader): void {
    Object.assign(s.act, { load: createLoad(s), loadStatic: createLoadStatic(s) });
}

/** (Re)load the newest page and start following from where it ends. */
function createLoad(s: HistoryReader): () => Promise<void> {
    return async (): Promise<void> => {
        const seq = ++s.loadSeq;
        // A new cursor holds every event from now until it is settled below,
        // so nothing written during the read is lost or doubled.
        const c = s.act.newCursor();
        const current = (): boolean => seq === s.loadSeq && !s.disposed;
        clearTimeout(s.publishTimer);
        s.publishTimer = undefined;
        s.dirty = false;
        s.accLines = [];
        s.accStamps = [];
        s.parser = s.newParser(0);
        batch(() => {
            s.setStale(false);
            s.setLoading(true);
            s.setLoadError(null);
        });
        try {
            const countResp = await RpcApi.BlockfileLineCountCommand(
                TabRpcClient,
                { block_id: s.readBlockId, filename: "output" },
                { timeout: 5000 }
            );
            if (!current()) return;
            const total = countResp?.count ?? 0;
            if (total === 0) {
                c.settle("empty");
                batch(() => {
                    s.setTotalLines(0);
                    s.setHistoryOffset(0);
                    s.act.publishNow();
                    s.setLoading(false);
                });
                return;
            }

            const offset = Math.max(0, total - HISTORY_PAGE);
            const resp = await RpcApi.BlockfileReadRangeCommand(
                TabRpcClient,
                { block_id: s.readBlockId, filename: "output", offset, limit: total - offset },
                { timeout: 30_000 }
            );
            if (!current()) return;
            s.accLines = resp.lines ?? [];
            s.accStamps = s.stampsOf(resp);
            s.parser = s.newParser(offset);
            s.parser.feed(s.accLines, s.accStamps);
            batch(() => {
                s.act.publishNow();
                s.setHistoryOffset(Math.min(offset, typeof resp.total === "number" ? resp.total : offset));
                s.setTotalLines(typeof resp.total === "number" ? resp.total : total);
                s.setEndLine(typeof resp.total === "number" ? resp.total : total);
                s.setLoading(false);
            });
            c.settle(historyPin(offset, resp));
            void s.act.fillPane(seq);
        } catch (err) {
            if (!current()) return;
            // Follow from the next record rather than not at all.
            c.settle(null);
            batch(() => {
                s.setLoadError((err as Error | undefined)?.message ?? String(err));
                s.setLoading(false);
            });
        }
    };
}

/**
 * Load what a jump or an archive shows: the page ending at line `end` (the
 * end when not given), without following. Read through `src()`, so an
 * archive's own lines.
 */
function createLoadStatic(s: HistoryReader): (end?: number) => Promise<void> {
    return async (end?: number): Promise<void> => {
        const seq = ++s.loadSeq;
        s.cursor?.dispose();
        s.cursor = null;
        const current = (): boolean => seq === s.loadSeq && !s.disposed;
        clearTimeout(s.publishTimer);
        s.publishTimer = undefined;
        s.dirty = false;
        s.accLines = [];
        s.accStamps = [];
        s.parser = s.newParser(0);
        batch(() => {
            s.setStale(false);
            s.setLoading(true);
            s.setLoadError(null);
        });
        try {
            const countResp = await RpcApi.BlockfileLineCountCommand(
                TabRpcClient,
                { block_id: s.readBlockId, filename: "output", ...s.src() },
                { timeout: 15_000 }
            );
            if (!current()) return;
            const total = countResp?.count ?? 0;
            const upTo = Math.min(end ?? total, total);
            const offset = Math.max(0, upTo - HISTORY_PAGE);
            const resp =
                upTo > offset
                    ? await RpcApi.BlockfileReadRangeCommand(
                          TabRpcClient,
                          { block_id: s.readBlockId, filename: "output", offset, limit: upTo - offset, ...s.src() },
                          { timeout: 30_000 }
                      )
                    : { lines: [] as string[] };
            if (!current()) return;
            s.accLines = resp.lines ?? [];
            s.accStamps = s.stampsOf(resp);
            s.parser = s.newParser(offset);
            s.parser.feed(s.accLines, s.accStamps);
            batch(() => {
                s.act.publishNow();
                s.setHistoryOffset(offset);
                s.setEndLine(offset + s.accLines.length);
                s.setTotalLines(total);
                s.setLoading(false);
            });
            queueMicrotask(() => s.scrollToBottom?.("history-jump"));
            void s.act.fillPane(seq);
        } catch (err) {
            if (!current()) return;
            batch(() => {
                s.setLoadError((err as Error | undefined)?.message ?? String(err));
                s.setLoading(false);
            });
        }
    };
}
