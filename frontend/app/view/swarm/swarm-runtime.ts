// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The runtime state of every created agent, for the Swarm's rows of agents that have no pane.
// `agentruntime.list` gives the rows, and `agentruntime:changed:<agent id>` carries a change to one
// (srv `server/agent_runtime.rs`). An event is named by the agent, so a subscription is kept for each
// agent the list has, and a list is read again when the agents change (`agents:changed`) to follow
// an agent created or deleted: srv sends nothing for a deleted one.

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createSignal, type Accessor } from "solid-js";

export interface AgentRuntimeDeps {
    list: () => Promise<AgentRuntime[]>;
    /** Follow one agent's changes; returns the unsubscribe. */
    subscribe: (agentId: string, handler: (row: AgentRuntime) => void) => () => void;
    /** Be told the agents changed (created, deleted, renamed); returns the unsubscribe. */
    onAgentsChanged: (handler: () => void) => () => void;
}

export interface AgentRuntimeStore {
    rows: Accessor<AgentRuntime[]>;
    /** Read the list, and follow the agents it names. */
    load: () => Promise<void>;
    dispose: () => void;
}

export function createAgentRuntimeStore(deps: AgentRuntimeDeps): AgentRuntimeStore {
    const [byId, setById] = createSignal<ReadonlyMap<string, AgentRuntime>>(new Map());
    const follows = new Map<string, () => void>();
    let reading: Promise<void> | null = null;
    let again = false;
    let disposed = false;

    const follow = (ids: Set<string>) => {
        for (const [id, unsubscribe] of follows) {
            if (ids.has(id)) continue;
            unsubscribe();
            follows.delete(id);
        }
        for (const id of ids) {
            if (follows.has(id)) continue;
            follows.set(
                id,
                deps.subscribe(id, (row) => setById((prev) => new Map(prev).set(row.agent_id, row)))
            );
        }
    };

    const read = async () => {
        try {
            const rows = await deps.list();
            if (disposed) return;
            setById(new Map(rows.map((r) => [r.agent_id, r] as const)));
            follow(new Set(rows.map((r) => r.agent_id)));
        } catch {
            // An srv without the command, or a transient failure: the Swarm shows the agents it has.
        }
    };

    // One read at a time; a change that arrives while one runs asks for one more after it.
    const load = (): Promise<void> => {
        if (reading) {
            again = true;
            return reading;
        }
        reading = (async () => {
            do {
                again = false;
                await read();
            } while (again && !disposed);
        })().finally(() => {
            reading = null;
        });
        return reading;
    };

    const unsubscribeAgents = deps.onAgentsChanged(() => void load());
    return {
        rows: () => [...byId().values()],
        load,
        dispose: () => {
            disposed = true;
            unsubscribeAgents();
            for (const unsubscribe of follows.values()) unsubscribe();
            follows.clear();
        },
    };
}

/** The agents to list as stopped: no process, and no row of their own in the list of live ones
 *  (`liveBlockIds`: the blocks the Swarm lists). Ordered by name. */
export function stoppedAgents(rows: readonly AgentRuntime[], liveBlockIds: ReadonlySet<string>): AgentRuntime[] {
    return rows
        .filter((r) => (r.state === "stopped" || r.state === "error") && !(r.block_id && liveBlockIds.has(r.block_id)))
        .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
}
