// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The header's one line about what the reader shows: its state, and which lines of how many. */

import type { HistoryMode } from "./HistoryNavigator";

export function historyMetaText(s: {
    loading: boolean;
    loadError: string | null;
    mode: HistoryMode;
    totalLines: number;
    historyOffset: number;
    endLine: number;
    /** Puts when the rows on screen were received in front of a line. */
    withSpan: (text: string) => string;
}): string {
    // Explicit loading state first — without it the initial async window renders
    // "no recorded history", which is indistinguishable from a genuinely empty agent.
    if (s.loading) return "loading history…";
    if (s.loadError) return `couldn't load history: ${s.loadError}`;
    if (s.mode !== "live" && s.totalLines > 0) {
        if (s.endLine <= s.historyOffset) return "nothing before this point";
        return s.withSpan(`${s.mode === "archive" ? "archived conversation · " : ""}lines ${s.historyOffset + 1}–${s.endLine} of ${s.totalLines}`);
    }
    if (s.historyOffset === 0 && s.totalLines > 0) return "full history loaded";
    if (s.totalLines > 0) return s.withSpan(`${s.totalLines} lines · scroll up for earlier`);
    return "no recorded history";
}
