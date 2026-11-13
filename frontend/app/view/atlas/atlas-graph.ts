// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Atlas's data: the Swarm's agents, as a graph (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §3 to §5).
 * Machines are regions, agents are nodes, and a line runs from an agent's lead to it ("reports to"; dashed
 * for a spawned worker). This file is pure: `atlas-model.ts` reads the Swarm's stores and hands the rows here.
 *
 * Stopped agents are not nodes: each region folds them into a chip, as the Swarm folds them into one section.
 */

import type { SeatSummary } from "@/types/rpc/SeatSummary";

/** What an agent is doing, in Atlas's own words: the Swarm's local and remote states folded into one set. */
export type AtlasState = "working" | "waiting" | "idle" | "error" | "limited" | "signed_out" | "stopped";

/** One agent, from this instance or another. */
export interface AtlasAgent {
    /** Unique in the graph: the Swarm's own row key. */
    key: string;
    name: string;
    provider: string | null;
    state: AtlasState;
    /** The Swarm's summary line for it, as it shows it. */
    line: string | null;
    /** The `AtlasMachine.key` it runs on. */
    machine: string;
    /** What a lead's id names: its uid, or a synthetic one for an agent of another instance. */
    agentId: string | null;
    seat: SeatSummary | null;
    contextTokens: number | null;
    currentTool: string | null;
    /** The pane it runs in on this instance, which Open focuses. Null for an agent of another instance or with no pane. */
    blockId: string | null;
    /** The runtime's agent id, for Open and Start of an agent with no pane (a stopped one). */
    runtimeId: string | null;
    remote: boolean;
    /** Its machine's last report is old: the state shown may not be current. */
    stale: boolean;
    /** Spent today against its daily cap, for an agent on a model route with one (this instance's only). */
    spend?: { spent: number; cap: number } | null;
}

/** One machine, or one channel of one. */
export interface AtlasMachine {
    key: string;
    title: string;
    /** How it was found: "this machine", "LAN" or "cloud". */
    badge: string;
    platform: string | null;
    stale: boolean;
    seenAtMs: number | null;
}

/** A line from a lead to the agent that reports to it. */
export interface AtlasEdge {
    from: string;
    to: string;
    /** "spawned": a worker its lead started, rather than one given a seat. */
    kind: "lead" | "spawned";
}

export interface AtlasRegion extends AtlasMachine {
    /** The agents drawn as nodes, in the order they were given. */
    shown: AtlasAgent[];
    /** The stopped ones, folded into the region's chip. */
    folded: AtlasAgent[];
}

export interface AtlasGraph {
    regions: AtlasRegion[];
    edges: AtlasEdge[];
}

/** A lead written `Name@host`: its name and the machine it is on. */
function splitLeadName(name: string): { name: string; host: string | null } {
    const at = name.lastIndexOf("@");
    if (at <= 0 || at === name.length - 1) return { name: name.trim(), host: null };
    return { name: name.slice(0, at).trim(), host: name.slice(at + 1).trim() };
}

const same = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "base" }) === 0;

/** Whether a machine's title is the host a lead names ("narko", or "narko · dev" for one of its channels). */
function onHost(machine: AtlasMachine | undefined, host: string): boolean {
    if (!machine) return false;
    const first = machine.title.split("·")[0].trim();
    return same(first, host);
}

/**
 * The shown agent a seat's lead names, or null. By id first (this instance's agents carry their uid; those of another
 * instance a synthetic id the Swarm gives them within their section). Then by name: `Name@host` names an agent on
 * that machine, and a bare name the one on the lead's own machine, or the only one of that name anywhere. A lead it
 * cannot find, or cannot tell from another, is no line, never a wrong one.
 */
function resolveLead(agent: AtlasAgent, shown: readonly AtlasAgent[], machines: ReadonlyMap<string, AtlasMachine>): AtlasAgent | null {
    const lead = agent.seat?.lead;
    if (!lead) return null;
    const byId = shown.find((a) => a !== agent && a.agentId !== null && a.agentId === lead.id);
    if (byId) return byId;
    const named = lead.name ? splitLeadName(lead.name) : null;
    if (!named?.name) return null;
    const candidates = shown.filter((a) => a !== agent && same(a.name, named.name));
    if (named.host) {
        const there = candidates.filter((a) => onHost(machines.get(a.machine), named.host!));
        return there.length === 1 ? there[0] : null;
    }
    const nearby = candidates.filter((a) => a.machine === agent.machine);
    if (nearby.length === 1) return nearby[0];
    return candidates.length === 1 ? candidates[0] : null;
}

/**
 * The graph of `agents` on `machines`. A machine with no agents is left out; machines keep the order given (this one
 * first). An edge joins an agent to its lead when both are shown; a lead that is stopped, absent or unknown leaves the
 * agent a root.
 */
export function buildAtlasGraph(machines: readonly AtlasMachine[], agents: readonly AtlasAgent[]): AtlasGraph {
    const byKey = new Map(machines.map((m) => [m.key, m] as const));
    const regions: AtlasRegion[] = [];
    for (const machine of machines) {
        const own = agents.filter((a) => a.machine === machine.key);
        if (own.length === 0) continue;
        regions.push({
            ...machine,
            shown: own.filter((a) => a.state !== "stopped"),
            folded: own.filter((a) => a.state === "stopped"),
        });
    }
    const shown = regions.flatMap((r) => r.shown);
    const edges: AtlasEdge[] = [];
    for (const agent of shown) {
        const lead = resolveLead(agent, shown, byKey);
        if (!lead) continue;
        edges.push({ from: lead.key, to: agent.key, kind: agent.seat?.qualifier === "spawned" ? "spawned" : "lead" });
    }
    return { regions, edges };
}
