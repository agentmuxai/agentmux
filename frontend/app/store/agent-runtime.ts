// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The runtime state of every created agent: the Swarm's rows of agents that have no pane, the status
// bar's counts and My Agents' state chips.
// `agentruntime.list` gives the rows, and `agentruntime:changed:<agent id>` carries a change to one
// (srv `server/agent_runtime.rs`). An event is named by the agent, so a subscription is kept for each
// agent the list has. srv sends nothing for an agent created or deleted, so the list is read again
// when the agents change (`agents:changed`) and on a slow interval, which also covers the paths that
// publish no such event (an instance created or deleted on its own).

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createSignal, type Accessor } from "solid-js";

/** How often the list is read again, for an agent created or deleted without an `agents:changed`. */
const RECONCILE_EVERY_MS = 20_000;

export interface AgentRuntimeDeps {
    list: () => Promise<AgentRuntime[]>;
    /** Follow one agent's changes; returns the unsubscribe. */
    subscribe: (agentId: string, handler: (row: AgentRuntime) => void) => () => void;
    /** Be told the agents changed (created, deleted, renamed); returns the unsubscribe. */
    onAgentsChanged: (handler: () => void) => () => void;
    /** Every `ms`, until the returned stop is called. */
    every: (ms: number, tick: () => void) => () => void;
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
    // The agents an event changed while a read was out: the read is older than that event.
    const changedMeanwhile = new Set<string>();
    let reading: Promise<void> | null = null;
    let inFlight = false;
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
                deps.subscribe(id, (row) => {
                    if (inFlight) changedMeanwhile.add(row.agent_id);
                    setById((prev) => new Map(prev).set(row.agent_id, row));
                })
            );
        }
    };

    const read = async () => {
        changedMeanwhile.clear();
        inFlight = true;
        try {
            const rows = await deps.list();
            if (disposed) return;
            const current = byId();
            const next = new Map(rows.map((r) => [r.agent_id, r] as const));
            // What an event said while the list was out is newer than the list.
            for (const id of changedMeanwhile) {
                const newer = current.get(id);
                if (newer && next.has(id)) next.set(id, newer);
            }
            setById(next);
            follow(new Set(rows.map((r) => r.agent_id)));
        } catch {
            // An srv without the command, or a transient failure: the Swarm shows the agents it has.
        } finally {
            inFlight = false;
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
    const stopEvery = deps.every(RECONCILE_EVERY_MS, () => void load());
    return {
        rows: () => [...byId().values()],
        load,
        dispose: () => {
            disposed = true;
            unsubscribeAgents();
            stopEvery();
            for (const unsubscribe of follows.values()) unsubscribe();
            follows.clear();
        },
    };
}

/** An agent has a process from its start until it stops: starting, idle, working, or waiting on the
 *  user. (One held by a limit or a sign-in it can't pass has none to count.) */
const RUNNING: ReadonlySet<AgentRuntime["state"]> = new Set(["starting", "idle", "working", "waiting_on_user"]);

export const hasProcess = (state: AgentRuntime["state"]): boolean => RUNNING.has(state);

export interface AgentRuntimeCounts {
    /** Agents with a process, which is what costs: closing a pane no longer stops one. */
    running: number;
    /** Of those, the ones asking something of the user. */
    waiting: number;
}

export function runtimeCounts(rows: readonly AgentRuntime[]): AgentRuntimeCounts {
    let running = 0;
    let waiting = 0;
    for (const r of rows) {
        if (!hasProcess(r.state)) continue;
        running++;
        if (r.state === "waiting_on_user") waiting++;
    }
    return { running, waiting };
}
