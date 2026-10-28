// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";

/** The agents to list as stopped: no process, and no row of their own in the list of live ones
 *  (`liveBlockIds`: the blocks the Swarm lists). Ordered by name. */
export function stoppedAgents(rows: readonly AgentRuntime[], liveBlockIds: ReadonlySet<string>): AgentRuntime[] {
    return rows
        .filter((r) => (r.state === "stopped" || r.state === "error") && !(r.block_id && liveBlockIds.has(r.block_id)))
        .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
}
