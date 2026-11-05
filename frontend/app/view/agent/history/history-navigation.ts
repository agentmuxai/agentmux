// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where the History reader looks: the next page after a jump or an archive, back to the latest, an
 * archive, a jump to a line, and a position asked for from outside. Split out of AgentHistoryView.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { batch, createEffect, untrack } from "solid-js";
import { parseHistoryAt, type HistoryAt } from "./history-start-at";
import { HISTORY_PAGE, type HistoryReader } from "./history-reader-state";

export function useHistoryNavigation(s: HistoryReader): void {
    /** The next page after what a jump or an archive shows. */
    const loadNewer = async (): Promise<void> => {
        if (untrack(s.mode) === "live" || s.loadingOlder()) return;
        const from = s.endLine();
        const total = s.totalLines();
        if (from >= total) return;
        const seq = s.loadSeq;
        s.setLoadingOlder(true);
        try {
            const resp = await RpcApi.BlockfileReadRangeCommand(
                TabRpcClient,
                {
                    block_id: s.readBlockId,
                    filename: "output",
                    offset: from,
                    limit: Math.min(HISTORY_PAGE, total - from),
                    ...s.src(),
                },
                { timeout: 15_000 }
            );
            if (seq !== s.loadSeq || s.disposed) return;
            const lines = resp.lines ?? [];
            const stamps = s.stampsOf(resp);
            s.parser.feed(lines, stamps);
            s.accLines = s.accLines.concat(lines);
            s.accStamps = s.accStamps.concat(stamps);
            batch(() => {
                s.act.publishNow();
                s.setEndLine(from + lines.length);
            });
        } catch {
            // Non-fatal, as loadOlder: the button stays.
        } finally {
            s.setLoadingOlder(false);
        }
    };

    /** Back to following the current conversation, from its newest page. */
    const backToLatest = async (): Promise<void> => {
        batch(() => {
            s.setMode("live");
            s.setArchiveZone(null);
        });
        await s.act.load();
        s.scrollToBottom?.("history-back-to-latest");
    };

    const openArchive = (zone: string | null): void => {
        if (!zone) return void backToLatest();
        batch(() => {
            s.setArchiveZone(zone);
            s.setMode("archive");
        });
        void s.act.loadStatic();
    };

    const jumpTo = (line: number): void => {
        if (untrack(s.mode) !== "archive") s.setMode("jumped");
        void s.act.loadStatic(line);
    };

    // The Audit's "In history": scroll to the message. A byte locates exactly, a time to the nearest line.
    let appliedAt = -1;
    const openAt = async (at: HistoryAt): Promise<void> => {
        const archive = at.zone && !at.zone.endsWith(":current") ? at.zone : null;
        try {
            const loc = await RpcApi.HistoryLocateCommand(TabRpcClient, {
                block_id: s.readBlockId,
                ...(archive ? { archive_zone: archive } : {}),
                ...(at.at_byte !== undefined ? { at_byte: at.at_byte } : { at_ms: at.at_ms }),
            });
            if (s.disposed) return;
            batch(() => {
                s.setArchiveZone(archive);
                s.setMode(archive ? "archive" : "jumped");
            });
            void s.act.loadStatic(loc.line);
        } catch (err) {
            if (!s.disposed) s.setLoadError((err as Error | undefined)?.message ?? String(err));
        } finally {
            s.props.onStartAtApplied?.();
        }
    };
    createEffect(() => {
        const at = parseHistoryAt(s.props.startAt?.());
        if (!at || at.n === appliedAt) return;
        appliedAt = at.n;
        void openAt(at);
    });
    Object.assign(s.act, { loadNewer, backToLatest, openArchive, jumpTo });
}
