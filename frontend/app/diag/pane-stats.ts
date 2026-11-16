// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * What each mounted agent pane holds, for the periodic renderer line
 * (renderer-state-diagnostic.ts): the rows of its loaded transcript and whether
 * its find bar is open. A pane registers a read when it mounts and removes it
 * when it goes, so the line costs a few reads every five minutes and nothing
 * per row or per frame. Counts only: no text, titles or paths (task #47823,
 * docs/specs/SPEC_LONG_RUNNING_ZERO_DEGRADATION_2026_11_16.md §4).
 */

/** 0: the bar is closed; 1: open with nothing typed; 2: open with a query. */
export type SearchState = 0 | 1 | 2;

export interface PaneStats {
    /** Rows in the pane's loaded transcript. */
    rows: number;
    search: SearchState;
}

const panes = new Map<string, () => PaneStats>();

/** A mounted pane says how to read its counts; the returned function removes it. */
export function registerPaneStats(blockId: string, read: () => PaneStats): () => void {
    panes.set(blockId, read);
    return () => {
        if (panes.get(blockId) === read) panes.delete(blockId);
    };
}

/** The search state for a bar's visibility and what is typed in it. */
export function searchState(visible: boolean, typed: string | undefined): SearchState {
    if (!visible) return 0;
    return typed !== undefined && typed.length > 0 ? 2 : 1;
}

export interface PaneTotals {
    panes: number;
    rows: number;
    /** Panes whose find bar is open. */
    searches: number;
    /** The largest pane's rows. */
    maxRows: number;
}

/** Each mounted pane's counts, largest transcript first. */
export function collectPaneStats(): Array<PaneStats & { blockId: string }> {
    const out: Array<PaneStats & { blockId: string }> = [];
    for (const [blockId, read] of panes) {
        try {
            out.push({ blockId, ...read() });
        } catch {
            // A pane tearing down must not cost the line.
        }
    }
    return out.sort((a, b) => b.rows - a.rows);
}

export function paneTotals(stats: ReadonlyArray<PaneStats> = collectPaneStats()): PaneTotals {
    let rows = 0;
    let searches = 0;
    let maxRows = 0;
    for (const s of stats) {
        rows += s.rows;
        if (s.search > 0) searches++;
        if (s.rows > maxRows) maxRows = s.rows;
    }
    return { panes: stats.length, rows, searches, maxRows };
}
