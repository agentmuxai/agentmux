// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Atlas's data: the Swarm's agents, as a graph (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §3 to §5).
 * Machines are regions, agents are nodes, and a line runs from an agent's lead to it ("reports to"; dashed
 * for a spawned worker). This file is pure: `atlas-model.ts` reads the Swarm's stores and hands the rows here.
 *
 * Stopped agents are not nodes: each region folds them into a chip, as the Swarm folds them into one section.
 *
 * Three things that are not agents are on the map too (§6.4): the container an agent runs in, as a fact of its
 * node; a container of this machine that no agent uses, as a node in its region; and the review bot and CI, as
 * two nodes of their own.
 */

import type { SeatSummary } from "@/types/rpc/SeatSummary";
import type { RuntimeKind } from "@/util/runtime-kind";

/** What an agent is doing, in Atlas's own words: the Swarm's local and remote states folded into one set. */
export type AtlasState = "working" | "waiting" | "idle" | "error" | "limited" | "signed_out" | "stopped";

/** One of this machine's agent containers, as srv lists it (`GET /api/v1/containers`). */
export interface AtlasContainer {
    name: string;
    state: "starting" | "running" | "stopped";
    /** The image it was made from: name and tag. */
    image: string;
    /** When it entered `state`, in ms since 1970. */
    sinceMs: number;
}

/** A container of a machine that no agent uses: a node in that machine's region. */
export interface AtlasOrphan extends AtlasContainer {
    /** Unique in the graph. */
    key: string;
    /** The `AtlasMachine.key` it is on. */
    machine: string;
}

/** One pull request's line at a service's node, for its card. */
export interface AtlasServiceLine {
    id: string;
    /** The pull request, as "#12". */
    pr: string;
    /** What became of it, in words: "approved", "changes requested", "failing". */
    state: string;
    tone: "passed" | "failed" | "neutral";
    /** The agent whose pull request it is, by the name messages reach it by. */
    agent: string;
    atMs: number;
}

/** The review bot or CI: a node that is not an agent, with a line to each agent whose pull request it worked on. */
export interface AtlasService {
    /** Unique in the graph. */
    key: string;
    name: string;
    icon: string;
    /** "warn": a pull request needs changes or fails its checks. "paused": its notices are not reaching this machine. */
    state: "ok" | "warn" | "paused";
    /** The line under its name. */
    sub: string;
    /** The pull requests it has a line for, newest first. */
    lines: AtlasServiceLine[];
}

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
    /** The name messages reach it by, when that is not its name (a renamed agent keeps its first one). */
    route?: string | null;
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
    /** Where it runs: on its machine or in a container. Absent for an agent of another instance, which does not say. */
    runtime?: RuntimeKind | null;
    /** The account an agent of another machine runs on, when its machine reported one. `label` is this
     *  computer's name for it, null when the account is not signed in here. */
    account?: { id: string; label: string | null } | null;
    /** The container it runs in, for one of this machine's container agents whose container srv lists. */
    container?: AtlasContainer | null;
}

/** The other agents that run on `agent`'s account, by name: they use up the same quota. */
export function accountSharers(agent: AtlasAgent, agents: Iterable<AtlasAgent>): string[] {
    const id = agent.account?.id;
    if (!id) return [];
    const names = [...agents].filter((other) => other.key !== agent.key && other.account?.id === id).map((other) => other.name);
    return names.sort((a, b) => a.localeCompare(b));
}

/** One machine, or one channel of one. */
export interface AtlasMachine {
    key: string;
    title: string;
    /** How it was found: "this machine", "LAN" or "cloud". */
    badge: string;
    /** Found through the account and not on the LAN: one of the account's other machines. Drawn after the others. */
    cloud?: boolean;
    platform: string | null;
    stale: boolean;
    seenAtMs: number | null;
    /** This computer, before its host name is known: its title is a placeholder, not a name. */
    unnamed?: boolean;
    /** Something wrong with the machine itself, said after its badge: its container runtime is not running. */
    note?: string;
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
    /** Its containers that no agent uses, drawn as nodes after the agents. */
    orphans: AtlasOrphan[];
}

export interface AtlasGraph {
    regions: AtlasRegion[];
    edges: AtlasEdge[];
    /** The review bot and CI, once either has been heard from. */
    services: AtlasService[];
}

/** What is on the map and is not an agent. */
export interface AtlasExtras {
    orphans?: readonly AtlasOrphan[];
    services?: readonly AtlasService[];
}

/** A lead written `Name@host`: its name and the machine it is on. */
function splitLeadName(name: string): { name: string; host: string | null } {
    const at = name.lastIndexOf("@");
    if (at <= 0 || at === name.length - 1) return { name: name.trim(), host: null };
    return { name: name.slice(0, at).trim(), host: name.slice(at + 1).trim() };
}

const same = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "base" }) === 0;

/** The computer a machine is, lowercased ("narko" for "narko", and for "narko · dev", one of its channels); null when not known. */
export function machineHost(machine: AtlasMachine | undefined): string | null {
    if (!machine || machine.unnamed) return null;
    return machine.title.split("·")[0].trim().toLowerCase() || null;
}

/** Whether a machine is the host a lead names. */
function onHost(machine: AtlasMachine | undefined, host: string): boolean {
    const name = machineHost(machine);
    return name !== null && same(name, host);
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
 * The graph of `agents` on `machines`. A machine with no agents and no container of its own is left out; machines
 * keep the order given (this one first). An edge joins an agent to its lead when both are shown; a lead that is
 * stopped, absent or unknown leaves the agent a root.
 */
export function buildAtlasGraph(
    machines: readonly AtlasMachine[],
    agents: readonly AtlasAgent[],
    extras: AtlasExtras = {}
): AtlasGraph {
    const byKey = new Map(machines.map((m) => [m.key, m] as const));
    const regions: AtlasRegion[] = [];
    for (const machine of machines) {
        const own = agents.filter((a) => a.machine === machine.key);
        const orphans = (extras.orphans ?? []).filter((o) => o.machine === machine.key);
        if (own.length === 0 && orphans.length === 0) continue;
        regions.push({
            ...machine,
            shown: own.filter((a) => a.state !== "stopped"),
            folded: own.filter((a) => a.state === "stopped"),
            orphans,
        });
    }
    const shown = regions.flatMap((r) => r.shown);
    const edges: AtlasEdge[] = [];
    for (const agent of shown) {
        const lead = resolveLead(agent, shown, byKey);
        if (!lead) continue;
        edges.push({ from: lead.key, to: agent.key, kind: agent.seat?.qualifier === "spawned" ? "spawned" : "lead" });
    }
    return { regions, edges, services: [...(extras.services ?? [])] };
}
