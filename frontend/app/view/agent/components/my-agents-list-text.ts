// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The text and the sort order of the My agents list: pure, so they sit apart from the component
// (SPEC_AGENT_PANE_CLEANUP: the list's helpers out of MyAgentsList.tsx). MyAgentsList re-exports
// the strings its tests and the picker match on.

import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { compareLastUsed } from "../agent-recency";
import type { AgentSortOption } from "./AgentPickerFilterBar";

/** "type" sort groups Native (stored as "host") before Container before anything
 *  unrecognized, matching `RuntimeBadge`'s own known-runtime ordering —
 *  not alphabetical ("container" < "host" would put Container first, which
 *  reads backwards next to the badge's own NATIVE/CONTAINER vocabulary).
 *
 *  Codex P2, PR #2789: `"standalone"` is the LEGACY default `agent_type`
 *  (`default_agent_type()`, `backend/storage/agents/mod.rs`) for definitions
 *  predating the container feature, and every non-`"container"` value is
 *  treated as the host controller at launch (`agent_open.rs`'s
 *  `controller_type = if agent.agent_type == "container" {...} else {...}`)
 *  — "standalone" IS a host agent, effectively, just under an older name.
 *  Without this, legacy definitions would rank as "unknown" (last),
 *  splitting them from genuinely host-labeled agents instead of grouping
 *  correctly with them. */
const TYPE_SORT_RANK: Record<string, number> = { host: 0, standalone: 0, container: 1 };
const typeSortRank = (agentType: string): number => TYPE_SORT_RANK[agentType] ?? 2;

export function compareRows(sort: AgentSortOption, a: RecentSessionRow, b: RecentSessionRow): number {
    switch (sort) {
        case "name":
            return (a.instance_name || a.definition_name).localeCompare(
                b.instance_name || b.definition_name,
                undefined,
                {
                    sensitivity: "base",
                }
            );
        case "type": {
            const rankDiff = typeSortRank(a.agent_type) - typeSortRank(b.agent_type);
            if (rankDiff !== 0) return rankDiff;
            return (a.instance_name || a.definition_name).localeCompare(
                b.instance_name || b.definition_name,
                undefined,
                {
                    sensitivity: "base",
                }
            );
        }
        case "recent":
        default:
            return compareLastUsed(a, b);
    }
}

/** Empty-state copy varies based on whether an identity filter was
 * applied — surfaced so the integration test can match it. */
export const EMPTY_GLOBAL = "No agents yet — pick a template below to create your first one.";
export const EMPTY_FILTERED = "No agents for this identity yet.";
/** Shown when ListRecentSessionsCommand itself failed — distinct from
 * EMPTY_GLOBAL/EMPTY_FILTERED so a backend error never looks identical
 * to "you genuinely have zero agents" (retro
 * docs/retro/retro-my-agents-fresh-channel-regression-2026-07-27.md §4/§9
 * rec 2 — this exact ambiguity is what let a real regression, PR #2296,
 * go unnoticed as "expected empty state" the first time it happened). */
export const FETCH_ERROR = "Couldn't load your agents — check the connection and try again.";
/** Copy for "the fetch succeeded, but the name filter matched nothing" —
 * distinct from EMPTY_GLOBAL/EMPTY_FILTERED (both mean "there is nothing
 * to filter"), so a user narrowing a real, non-empty list to zero visible
 * rows isn't told they have no agents at all.
 * SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md. */
export const noMatchText = (query: string): string => `No agents match "${query}".`;

/**
 * Copy for a row with no preview text. The history comes from the agent's
 * global transcript, so the cases are about that transcript, not about this
 * channel:
 *
 * - `snapshot_check_failed` — the transcript could not be read; the real state
 *   is unknown, not confirmed empty.
 * - `has_snapshot` — a transcript exists but holds no user prompt in the part
 *   that was searched (a long run of tool work).
 * - otherwise — the agent has no transcript at all.
 *
 * SPEC_MY_AGENTS_TILES_AUTH_AND_HISTORY_2026_10_03.md §5.3.
 */
export const noPreviewText = (row: Pick<RecentSessionRow, "snapshot_check_failed" | "has_snapshot">): string => {
    if (row.snapshot_check_failed) return "Couldn't read history";
    if (row.has_snapshot) return "No prompt in recent history";
    return "No conversations yet";
};
