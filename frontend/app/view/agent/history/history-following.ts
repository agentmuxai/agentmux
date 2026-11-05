// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The History reader following the transcript (spec §6.9): records that land after the load, the
 * cursor that places them, and recovery from a hole without yanking the reader. Split out of
 * AgentHistoryView.
 */

import { splitRecords, type TranscriptCursor } from "../transcript-cursor";
import { createHistoryCursor } from "./history-cursor";
import type { HistoryReader } from "./history-reader-state";
import { untrack } from "solid-js";

export function useHistoryFollowing(s: HistoryReader): void {
    // ---- Recovering from a hole without yanking the reader (spec §6.9) ----
    // After a gap History can't fill, appends would land after a hole, so
    // they stop being shown. A reader following the bottom gets the newest
    // page at once; one reading older turns keeps their pages and position,
    // and the reload waits until they ask for it or scroll back down.
    const recover = (): void => {
        if (untrack(s.mode) !== "live") return;
        if (untrack(s.following)) return void s.act.load();
        s.setStale(true);
    };
    const jumpToLatest = async (): Promise<void> => {
        await s.act.load();
        s.scrollToBottom?.("history-jump-to-latest");
    };

    // ---- Following the transcript ----
    // Records land here in order, exactly once, after the initial load
    // (TranscriptCursor). Echoes are the user's own messages — the live pane
    // pairs them with its optimistic copy; this reader has none, so they are
    // parsed like any other record.
    const appendRecords = (text: string): void => {
        if (untrack(s.stale) || untrack(s.mode) !== "live") return;
        // The cursor joined a new generation (an archive or restore that
        // recreated the stream without a replace event for this block): these
        // records start another transcript, which must not be parsed onto
        // this one's nodes.
        if ((s.cursor?.stats.genChanges ?? 0) > s.genChangesSeen) {
            s.genChangesSeen = s.cursor?.stats.genChanges ?? 0;
            recover();
            return;
        }
        const lines = splitRecords(text);
        if (lines.length === 0) return;
        const now = Date.now();
        const stamps = lines.map(() => now);
        try {
            s.parser.feed(lines, stamps);
        } catch (err) {
            // The cursor has already moved past these records and would only
            // log the throw: without this, they would be missing from what
            // History shows. Treat it like a skipped gap (spec §6.9).
            console.warn("[AgentHistoryView] parse failed; reloading", err);
            recover();
            return;
        }
        s.accLines = s.accLines.concat(lines);
        s.accStamps = s.accStamps.concat(stamps);
        s.setTotalLines((t) => t + lines.length);
        s.act.publishSoon();
    };

    /**
     * The cursor skips a gap it won't fill (over GAP_FILL_MAX_LINES, a failed
     * read, a vanished generation) by moving past it. Here that would leave
     * lines missing BELOW the loaded range, where loadOlder can't reach them,
     * so any skip reloads the newest page instead (spec §6.9).
     */
    const reloadIfSkipped = (): void => {
        const skipped = s.cursor?.stats.linesSkipped ?? 0;
        if (skipped <= s.skippedSeen) return;
        s.skippedSeen = skipped;
        recover();
    };
    const newCursor = (): TranscriptCursor => {
        s.cursor?.dispose();
        s.skippedSeen = 0;
        s.genChangesSeen = 0;
        const c = createHistoryCursor({
            readBlockId: s.readBlockId,
            deliver: appendRecords,
            reset: () => void s.act.load(),
            // Skips are logged as warnings; check once the log call's own bookkeeping has run.
            onSkipWarning: () => queueMicrotask(reloadIfSkipped),
        });
        s.cursor = c;
        return c;
    };
    Object.assign(s.act, { recover, jumpToLatest, appendRecords, reloadIfSkipped, newCursor });
}
