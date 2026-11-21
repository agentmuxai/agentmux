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
import { batch, createMemo, createSignal, untrack, type Accessor } from "solid-js";

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
    /**
     * Every agent's row. It is read again when any agent changes, so it is for what is about every agent (the status bar's
     * counts, the Swarm's agents without a pane); what is about one agent reads `rowOf` or `rowOfBlock`.
     */
    rows: Accessor<AgentRuntime[]>;
    /** One agent's row: it is read again only when that agent's row changed. Undefined for an agent the list does not have. */
    rowOf: (agentId: string) => AgentRuntime | undefined;
    /** The row of the agent whose pane is `blockId`: read again when that agent's row changed, or the pane's agent did. */
    rowOfBlock: (blockId: string) => AgentRuntime | undefined;
    /** Read the list, and follow the agents it names. */
    load: () => Promise<void>;
    dispose: () => void;
}

/** A row says the same as another: every field of a row is a plain value. */
export function sameRow(a: AgentRuntime | undefined, b: AgentRuntime | undefined): boolean {
    if (a === b) return true;
    if (!a || !b) return false;
    const ka = Object.keys(a) as (keyof AgentRuntime)[];
    if (ka.length !== Object.keys(b).length) return false;
    return ka.every((k) => Object.is(a[k], b[k]));
}

const sameIds = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((id, i) => id === b[i]);

const sameIndex = (a: ReadonlyMap<string, string>, b: ReadonlyMap<string, string>) =>
    a.size === b.size && [...a].every(([block, agent]) => b.get(block) === agent);

export function createAgentRuntimeStore(deps: AgentRuntimeDeps): AgentRuntimeStore {
    // One signal for each agent's row, so an event for one agent wakes only the readers of that agent; a signal drops
    // an event that says what the row already says. `latest` is the same rows for what reads them outside a reaction.
    const slots = new Map<string, { get: Accessor<AgentRuntime | undefined>; set: (r: AgentRuntime | undefined) => void }>();
    const latest = new Map<string, AgentRuntime>();
    const slotOf = (id: string) => {
        let slot = slots.get(id);
        if (!slot) {
            const [get, set] = createSignal<AgentRuntime | undefined>(undefined, { equals: sameRow });
            slot = { get, set: (r) => void set(() => r) };
            slots.set(id, slot);
        }
        return slot;
    };
    const [ids, setIds] = createSignal<readonly string[]>([], { equals: sameIds });
    const list = createMemo(() => {
        const out: AgentRuntime[] = [];
        for (const id of ids()) {
            const r = slotOf(id).get();
            if (r) out.push(r);
        }
        return out;
    });
    const blockIndex = createMemo(
        () => new Map(list().flatMap((r) => (r.block_id ? [[r.block_id, r.agent_id] as const] : []))),
        undefined,
        { equals: sameIndex }
    );
    const follows = new Map<string, () => void>();
    // The agents an event changed while a read was out: the read is older than that event.
    const changedMeanwhile = new Set<string>();
    let reading: Promise<void> | null = null;
    let inFlight = false;
    let again = false;
    let disposed = false;

    const apply = (row: AgentRuntime) => {
        const prev = latest.get(row.agent_id);
        if (prev && sameRow(prev, row)) return;
        latest.set(row.agent_id, row);
        slotOf(row.agent_id).set(row);
    };

    const follow = (wanted: Set<string>) => {
        for (const [id, unsubscribe] of follows) {
            if (wanted.has(id)) continue;
            unsubscribe();
            follows.delete(id);
        }
        for (const id of wanted) {
            if (follows.has(id)) continue;
            follows.set(
                id,
                deps.subscribe(id, (row) => {
                    if (inFlight) changedMeanwhile.add(row.agent_id);
                    const known = latest.has(row.agent_id);
                    apply(row);
                    if (!known) setIds([...untrack(ids), row.agent_id]);
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
            const listed = new Map(rows.map((r) => [r.agent_id, r] as const));
            batch(() => {
                // What an event said while the list was out is newer than the list.
                for (const [id, r] of listed) {
                    const newer = changedMeanwhile.has(id) ? latest.get(id) : undefined;
                    apply(newer ?? r);
                }
                for (const id of [...latest.keys()]) {
                    if (listed.has(id)) continue;
                    latest.delete(id);
                    slotOf(id).set(undefined);
                }
                setIds(rows.map((r) => r.agent_id));
            });
            follow(new Set(listed.keys()));
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
        rows: list,
        rowOf: (agentId) => slotOf(agentId).get(),
        rowOfBlock: (blockId) => {
            const id = blockIndex().get(blockId);
            return id ? slotOf(id).get() : undefined;
        },
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

/** An agent has a process in every state but stopped and error (srv's `desired_of`): one held by a
 *  limit or a sign-in is alive and waiting, and costs when it resumes. */
const NO_PROCESS: ReadonlySet<AgentRuntime["state"]> = new Set(["stopped", "error"]);

export const hasProcess = (state: AgentRuntime["state"]): boolean => !NO_PROCESS.has(state);

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
