// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Atlas's data source: the Swarm's own agents (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §5.5), as `AtlasAgent`
 * rows on `AtlasMachine`s. No second data path: it reads what `useSwarmAgents` and the seat info read, so the map and
 * the list always agree, and the filter is the Swarm's.
 */

import { readSeatMeta } from "@/app/agent-model/seat-format";
import { unavailableOfMeta } from "@/app/store/agent-unavailable";
import { MOS } from "@/app/store/global";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { awaitsBrowserAnswer, isAwaitingUser } from "@/app/store/swarm-line";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { runtimeKindOfBlockMeta } from "@/util/runtime-kind";
import { createMemo, type Accessor } from "solid-js";
import type { AtlasFlow } from "./atlas-flow";
import { useAtlasFlows } from "./atlas-flow-store";
import { buildAtlasGraph, type AtlasAgent, type AtlasGraph, type AtlasMachine, type AtlasState } from "./atlas-graph";
import { useSwarmAgents, type SwarmAgents } from "../swarm/swarm-agent-list";
import { phaseToDisplayStatus } from "../swarm/swarm-display-status";
import { remoteAgentKey } from "../swarm/swarm-fleet-targets";
import type { AgentTreeNode, SwarmViewModel } from "../swarm/swarm-model";
import { remoteSections, type RemoteAgentState, type RemoteRow } from "../swarm/swarm-remote";
import { useSwarmSeats } from "../swarm/swarm-seats";
import { chipStatus } from "../swarm/swarm-status-chip";

/** The key of the machine this window's AgentMux runs on. */
export const LOCAL_MACHINE = "local";

const REMOTE_STATES: Record<RemoteAgentState, AtlasState> = {
    working: "working",
    waiting: "waiting",
    idle: "idle",
    stopped: "stopped",
    error: "error",
    limited: "limited",
    signed_out: "signed_out",
};

/** A remote agent's state; one its instance did not report, or this build does not know, reads as idle. */
export function remoteAtlasState(status: string | null | undefined): AtlasState {
    return status && Object.hasOwn(REMOTE_STATES, status) ? REMOTE_STATES[status as RemoteAgentState] : "idle";
}

const RUNTIME_STATES: Record<AgentRuntime["state"], AtlasState> = {
    stopped: "stopped",
    starting: "stopped",
    idle: "idle",
    working: "working",
    waiting_on_user: "waiting",
    limited: "limited",
    signed_out: "signed_out",
    error: "error",
};

export function runtimeAtlasState(state: AgentRuntime["state"]): AtlasState {
    return RUNTIME_STATES[state] ?? "stopped";
}

type Meta = Record<string, unknown> | undefined;

function spendOf(meta: Meta): AtlasAgent["spend"] {
    const s = meta?.["agent:spend_today"] as { spent?: unknown; cap?: unknown } | undefined;
    return typeof s?.spent === "number" && typeof s.cap === "number" && s.cap > 0 ? { spent: s.spent, cap: s.cap } : null;
}

/** The name messages reach an agent by, as its launch put it in its pane's environment. */
function routeOf(meta: Meta): string | null {
    const id = (meta?.["cmd:env"] as Record<string, unknown> | undefined)?.["AGENTMUX_AGENT_ID"];
    return typeof id === "string" && id.trim() ? id.trim() : null;
}

/** One of this instance's agents, from its Swarm row and its pane's meta. */
function localAgent(key: string, node: AgentTreeNode, agentId: string | null, seat: AtlasAgent["seat"]): AtlasAgent {
    const meta: Meta = node.blockId ? MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", node.blockId))()?.meta : undefined;
    const display = node.blockId
        ? phaseToDisplayStatus(node.blockId, node.agentStatus)
        : node.agentStatus === "running"
          ? "working"
          : "idle";
    const chip = chipStatus(display, isAwaitingUser(meta, node.agentStatus), awaitsBrowserAnswer(meta, node.agentStatus));
    const live = chip !== "idle";
    const unavailable = unavailableOfMeta(meta, live, Date.now());
    const state: AtlasState =
        display === "error"
            ? "error"
            : chip === "question"
              ? "waiting"
              : chip === "working"
                ? "working"
                : (unavailable?.reason ?? "idle");
    return {
        key,
        name: node.agentName,
        provider: node.agentProvider,
        state,
        line: node.line.text || null,
        machine: LOCAL_MACHINE,
        agentId,
        route: routeOf(meta),
        seat,
        contextTokens: node.contextTokens,
        currentTool: node.currentTool,
        blockId: node.blockId,
        runtimeId: typeof meta?.["agentId"] === "string" ? (meta["agentId"] as string) : null,
        remote: false,
        stale: false,
        spend: spendOf(meta),
        runtime: node.blockId ? runtimeKindOfBlockMeta(meta) : null,
    };
}

/** A created agent that has no row: stopped, or running with no pane. */
function runtimeAgent(row: AgentRuntime): AtlasAgent {
    return {
        key: `runtime:${row.agent_id}`,
        name: row.name,
        provider: row.provider || null,
        state: runtimeAtlasState(row.state),
        line: null,
        machine: LOCAL_MACHINE,
        agentId: row.agent_id,
        seat: null,
        contextTokens: null,
        currentTool: null,
        blockId: row.block_id ?? null,
        runtimeId: row.agent_id,
        remote: false,
        stale: false,
    };
}

function remoteAgent(row: RemoteRow, key: string, agentId: string | null, seat: AtlasAgent["seat"]): AtlasAgent {
    const { agent, section } = row;
    return {
        key,
        name: agent.name,
        provider: agent.provider ?? null,
        state: remoteAtlasState(agent.status),
        line: agent.line?.trim() || null,
        machine: section.key,
        agentId,
        seat,
        contextTokens: null,
        currentTool: null,
        blockId: null,
        runtimeId: null,
        remote: true,
        stale: section.stale,
        account: agent.account ? { id: agent.account, label: agent.account_label?.trim() || null } : null,
    };
}

export interface AtlasData {
    graph: Accessor<AtlasGraph>;
    /** The Swarm's model: its selection and fleet actions are the map's too. */
    model: SwarmViewModel;
    /** The Swarm's list, for its filter bar. */
    swarm: SwarmAgents;
    /** Every agent, by key, for the card. */
    agents: Accessor<Map<string, AtlasAgent>>;
    machines: Accessor<Map<string, AtlasMachine>>;
    /** The messages and task hand-overs on show now, as srv reports them. */
    flows: Accessor<readonly AtlasFlow[]>;
}

/** The Swarm's agents as Atlas's graph, following the Swarm's own stores and its filter. */
export function useAtlasData(model: SwarmViewModel, ctx: PaneTabHostContext): AtlasData {
    const swarm = useSwarmAgents(model, ctx);
    const seats = useSwarmSeats(ctx, swarm.keyedRows, swarm.remoteByKey);

    const machines = createMemo<AtlasMachine[]>(() => {
        const other = model.otherInstancesAtom();
        const local: AtlasMachine = {
            key: LOCAL_MACHINE,
            title: other?.hostname || "This computer",
            badge: "this machine",
            platform: null,
            stale: false,
            seenAtMs: null,
            unnamed: !other?.hostname,
        };
        const sections = remoteSections(other).map<AtlasMachine>((s) => ({
            key: s.key,
            title: s.title,
            badge: s.badge,
            platform: s.platform,
            stale: s.stale,
            seenAtMs: s.seenAtMs,
        }));
        return [local, ...sections];
    });

    const agentList = createMemo<AtlasAgent[]>(() => {
        const shown = swarm.filter.shown();
        const info = seats.info();
        const out: AtlasAgent[] = [];
        for (const [key, node] of swarm.keyedRows()) {
            if (shown && !shown.has(key)) continue;
            const i = info.get(key);
            out.push(localAgent(key, node, i?.agentId ?? null, i?.seat ?? null));
        }
        // With a filter on, the Swarm's own list of the stopped agents that match; with none, all of them.
        for (const row of shown ? swarm.filter.stopped() : swarm.stopped()) out.push(runtimeAgent(row));
        for (const row of swarm.remote()) {
            const key = remoteAgentKey(row.section, row.agent);
            if (shown && !shown.has(key)) continue;
            const i = info.get(key);
            out.push(remoteAgent(row, key, i?.agentId ?? null, i?.seat ?? readSeatMeta({ "agent:seat": row.agent.seat })));
        }
        return out;
    });

    const graph = createMemo(() => buildAtlasGraph(machines(), agentList()));
    const agents = createMemo(() => new Map(agentList().map((a) => [a.key, a] as const)));
    const machineMap = createMemo(() => new Map(machines().map((m) => [m.key, m] as const)));
    return { graph, model, swarm, agents, machines: machineMap, flows: useAtlasFlows() };
}
