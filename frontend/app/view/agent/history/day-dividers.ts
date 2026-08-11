// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Day-separator injection for the Agent History view.
 *
 * Rule (spec §4.4): emit a `day_divider` row whenever the best-known
 * timestamp crosses a local-calendar-day boundary between consecutive
 * nodes, where best-known = the node's own timestamp if present, else the
 * last preceding known timestamp (carried forward). Untimestamped runs
 * group under the last known day rather than inventing times — for
 * pre-tsidx legacy content that resolves to "the day the surrounding
 * stamped records landed", which is honest and only ever applies to
 * pre-upgrade history.
 *
 * Dividers are render-time synthetics: recomputed over the full node list
 * on every change (the reader tops out at a few thousand nodes — one
 * linear pass), never persisted, never dispatched anywhere. Ids are
 * `day-<YYYY-MM-DD>` so a pagination prepend recomputes to the identical
 * node instead of duplicating.
 *
 * Spec: SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md §4.4.
 */

import type { DayDividerNode, DocumentNode } from "../types";

/** Local-calendar day key + label + midnight for a unix-ms timestamp. */
function dayOf(ms: number): { key: string; label: string; midnight: number } {
    const d = new Date(ms);
    const y = d.getFullYear();
    const m = d.getMonth();
    const day = d.getDate();
    const key = `${y}-${String(m + 1).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
    const label = d.toLocaleDateString(undefined, {
        weekday: "short",
        month: "short",
        day: "numeric",
        year: "numeric",
    });
    return { key, label, midnight: new Date(y, m, day).getTime() };
}

/**
 * Return `nodes` with `day_divider` rows inserted at every best-known
 * local-day change. Nodes with no known timestamp (absent or 0) inherit
 * the last seen day; leading untimestamped nodes (no day known yet)
 * produce no divider.
 *
 * Guarantees each `day-<YYYY-MM-DD>` id appears AT MOST ONCE in the output,
 * even if the day sequence isn't perfectly monotonic (real transcripts
 * aren't guaranteed to be: subagent transcript merges, tool-call-start vs.
 * log-flush timestamps, and retries/resumes can all produce a node stream
 * that briefly "goes back" a day before continuing forward). Without this,
 * a day visited twice (…Aug 10 → Aug 11 → Aug 10…) emitted TWO
 * `day-2026-08-10` divider rows with the identical id — a duplicate key in
 * the `<Key by={r => r.nodeId}>` virtualized list, which crashed
 * `reconcileArrays` ("replaceChild: node not a child") once real,
 * large-volume history started loading (live-reported right after
 * SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md shipped —
 * previously masked by the history reader silently reading through the
 * wrong, near-empty block, per that same doc's codex-P1 fix). A day that's
 * revisited after the sequence moves on simply gets no second divider —
 * its content still renders in the right visual position relative to its
 * neighbors; only the (already slightly dishonest, given the reordering)
 * *second* boundary marker is suppressed.
 */
export function injectDayDividers(nodes: ReadonlyArray<DocumentNode>): DocumentNode[] {
    const out: DocumentNode[] = [];
    let currentDayKey: string | null = null;
    const dividedDayKeys = new Set<string>();
    for (const node of nodes) {
        const ts = (node as { timestamp?: number }).timestamp;
        if (ts != null && ts > 0) {
            const day = dayOf(ts);
            if (day.key !== currentDayKey) {
                currentDayKey = day.key;
                if (!dividedDayKeys.has(day.key)) {
                    dividedDayKeys.add(day.key);
                    const divider: DayDividerNode = {
                        type: "day_divider",
                        id: `day-${day.key}`,
                        dayLabel: day.label,
                        timestamp: day.midnight,
                    };
                    out.push(divider);
                }
            }
        }
        out.push(node);
    }
    return out;
}
