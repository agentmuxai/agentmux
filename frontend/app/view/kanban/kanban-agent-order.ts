// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { MOS } from "@/app/store/global";
import { sortByLastUsed } from "@/app/view/agent/agent-recency";
import { lastActiveMs } from "@/app/view/swarm/swarm-last-active";
import { untrack } from "solid-js";
import type { KanbanAgent } from "./kanban-types";

/**
 * The agents a task can be given or assigned to, most recently used first
 * (agent-recency.ts): by when each last produced output, which its pane keeps
 * in `session:last_activity_ms`. `lastActive` is how to read that for an agent;
 * the default reads its pane's block.
 *
 * Read untracked: the pane stamps that time about once a second, and a picker
 * that reordered under the pointer would be worse than one that is current as
 * of when its options were last built.
 */
export function agentsByRecentUse(
    agents: readonly KanbanAgent[],
    lastActive: (a: KanbanAgent) => number = paneLastActive
): KanbanAgent[] {
    return untrack(() =>
        sortByLastUsed(
            agents.map((agent) => ({ agent, last_active_at: lastActive(agent) })),
            (x) => x.agent.name
        ).map((x) => x.agent)
    );
}

function paneLastActive(a: KanbanAgent): number {
    if (!a.block_id) return 0;
    const block = MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", a.block_id))();
    return lastActiveMs(block?.meta);
}
