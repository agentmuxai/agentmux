// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";

/** Every created agent the live rows do not list (`liveBlockIds`: the blocks the Swarm lists), by name: the
 *  stopped ones, and one that runs with no pane the list has no row for (started with no pane, or its block
 *  not read yet). An agent is in the live rows or here, never missing from both. */
export function agentsWithoutRow(rows: readonly AgentRuntime[], liveBlockIds: ReadonlySet<string>): AgentRuntime[] {
    return rows
        .filter((r) => !(r.block_id && liveBlockIds.has(r.block_id)))
        .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
}
