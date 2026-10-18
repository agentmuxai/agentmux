// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — History section: the pure parts, testable without a DOM.

import type { HistoryAgent } from "@/types/rpc/HistoryAgent";

/** A row of the agents list: an agent, and how deep it sits under the agent it was forked from. */
export interface AgentRow {
    agent: HistoryAgent;
    depth: number;
}

/**
 * The agents as a tree: each fork under the agent it was forked from, in the
 * order given (most recently active first). A fork whose parent has no history
 * of its own is a root. With a filter, the matching agents only, flat.
 */
export function agentRows(agents: HistoryAgent[], filter: string): AgentRow[] {
    const q = filter.trim().toLowerCase();
    if (q) {
        const match = (a: HistoryAgent) =>
            [a.name, a.definition_id, a.provider, a.fork_of_name].some((s) => s.toLowerCase().includes(q));
        return agents.filter(match).map((agent) => ({ agent, depth: 0 }));
    }
    const ids = new Set(agents.map((a) => a.definition_id));
    const children = new Map<string, HistoryAgent[]>();
    const roots: HistoryAgent[] = [];
    for (const a of agents) {
        if (a.fork_of && ids.has(a.fork_of) && a.fork_of !== a.definition_id) {
            children.set(a.fork_of, [...(children.get(a.fork_of) ?? []), a]);
        } else {
            roots.push(a);
        }
    }
    const rows: AgentRow[] = [];
    const seen = new Set<string>();
    const visit = (a: HistoryAgent, depth: number) => {
        if (seen.has(a.definition_id)) return;
        seen.add(a.definition_id);
        rows.push({ agent: a, depth });
        for (const c of children.get(a.definition_id) ?? []) visit(c, depth + 1);
    };
    roots.forEach((a) => visit(a, 0));
    // A cycle of forks has no root: list what the walk didn't reach.
    agents.forEach((a) => visit(a, 0));
    return rows;
}

/** What the agents list is summed up as: "12 agents · 3.4 GB · 5 archived conversations". */
export function historyTotals(agents: HistoryAgent[]): { agents: number; bytes: number; archives: number } {
    return agents.reduce(
        (t, a) => ({ agents: t.agents + 1, bytes: t.bytes + a.current_bytes + a.archive_bytes, archives: t.archives + a.archives }),
        { agents: 0, bytes: 0, archives: 0 }
    );
}

/** A Unix-ms time as a short local date and time, or "—" when unknown. */
export function formatWhen(ms: number): string {
    if (!ms) return "—";
    return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}
