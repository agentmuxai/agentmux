// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One row per agent. srv lists every block that has an agent controller, and an agent can have a second block:
// a pane from an older layout, closed at startup because the agent is open in another pane
// (`open_in_other_pane`, srv `agent_reconcile.rs`). That pane stays where it was, saying where the agent is, in a
// tab of a workspace that may have no window. It is not a second agent, so it gets no row while the pane that
// has the agent is listed: its name, tokens and a days-old status under the live row read as a second agent.

import { agentUidOfBlockMeta } from "./swarm-open-parked";

/** What tells one agent's blocks apart from another's, and a closed second pane from the pane that has the agent. */
export interface PaneIdentity {
    blockId: string;
    /** The agent the block runs (its instance, else its id); "" when the block names none. */
    agent: string;
    /** The pane is closed because the agent is open in another pane. */
    closedForAnotherPane: boolean;
    /** The pane srv said has the agent, when this one is closed for it. */
    holderBlockId: string;
}

export function paneIdentity(blockId: string, meta: Record<string, unknown> | null | undefined): PaneIdentity {
    const failure = meta?.["agent:last_failure"] as { code?: unknown; holderBlockId?: unknown } | null | undefined;
    const closed = failure?.code === "open_in_other_pane";
    return {
        blockId,
        agent: agentUidOfBlockMeta(meta ?? undefined),
        closedForAnotherPane: closed,
        holderBlockId: closed && typeof failure?.holderBlockId === "string" ? failure.holderBlockId : "",
    };
}

/**
 * The blocks that get no row: a pane closed for another pane, while another pane of the same agent is listed and
 * is not closed that way itself. A closed pane whose agent has no other row keeps its row, so the agent stays
 * reachable from the list. The pane it names decides only for a block that names no agent: that pane can since
 * have been given to another agent (the picker reuses a pane in place), and the mark is not rewritten then.
 */
export function closedSecondPanes(panes: readonly PaneIdentity[]): Set<string> {
    const holding = panes.filter((p) => !p.closedForAnotherPane);
    const holdingBlocks = new Set(holding.map((p) => p.blockId));
    const holdingAgents = new Set(holding.map((p) => p.agent).filter((a) => a !== ""));
    const hidden = new Set<string>();
    for (const p of panes) {
        if (!p.closedForAnotherPane) continue;
        const listedElsewhere = p.agent === "" ? holdingBlocks.has(p.holderBlockId) : holdingAgents.has(p.agent);
        if (listedElsewhere) hidden.add(p.blockId);
    }
    return hidden;
}
