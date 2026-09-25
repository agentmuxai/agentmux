// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Open Agent History" link-row injection for the LIVE working document — a
 * render-time synthetic (never persisted, never dispatched into the reducer
 * store), mirroring `history/day-dividers.ts`'s `injectDayDividers` pattern
 * for the history reader.
 *
 * Spec: SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.2.
 */

import type { DocumentNode, HistoryLinkNode } from "./types";

const HISTORY_LINK_NODE: HistoryLinkNode = { type: "history_link", id: "history-link" };

/**
 * Insert the history-link row right after the first node when it's a fresh
 * `session_outcome` divider (the normal clamp shape — see agent-view.tsx's
 * `earlierHistoryAvailable`), otherwise at the very front (defensive
 * fallback so the link still appears whenever the caller says earlier
 * history is available, even if the boundary node isn't literally first).
 * No-op — returns `nodes` unchanged — when `show` is false.
 */
export function injectHistoryLink(
    nodes: ReadonlyArray<DocumentNode>,
    show: boolean,
    opts?: { earlierTurns?: number },
): DocumentNode[] {
    if (!show) return nodes as DocumentNode[];
    const link: HistoryLinkNode =
        opts?.earlierTurns != null ? { ...HISTORY_LINK_NODE, earlierTurns: opts.earlierTurns } : HISTORY_LINK_NODE;
    const first = nodes[0];
    if (first?.type === "session_outcome" && first.outcome === "fresh") {
        return [first, link, ...nodes.slice(1)];
    }
    return [link, ...nodes];
}

/**
 * The live feed's gap rows (spec §6.9): where turns rolled off around one
 * that had to stay, a row before the first node after the gap says the
 * missing turns are in History. A gap before the first node is the top
 * link's job, so none is added there.
 */
export function injectGapRows(nodes: ReadonlyArray<DocumentNode>, gapsBefore: ReadonlySet<string>): DocumentNode[] {
    if (gapsBefore.size === 0) return nodes as DocumentNode[];
    const out: DocumentNode[] = [];
    for (let i = 0; i < nodes.length; i++) {
        const n = nodes[i];
        if (i > 0 && gapsBefore.has(n.id)) out.push({ type: "history_link", id: `history-gap:${n.id}`, gap: true });
        out.push(n);
    }
    return out;
}
