// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Atlas's data source: the Swarm's own agents (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §5.5), as `AtlasAgent`
 * rows on `AtlasMachine`s. No second data path: it reads what `useSwarmAgents` and the seat info read, so the map and
 * the list always agree, and the filter is the Swarm's. What is not an agent comes from where it lives: this
 * machine's containers from the container status store, the review bot and CI from the pull request lines.
 */

import { readSeatMeta } from "@/app/agent-model/seat-format";
import { unavailableOfMeta } from "@/app/store/agent-unavailable";
import { sharedContainerStatus } from "@/app/store/container-status-live";
import { MOS } from "@/app/store/global";
import { clockTime } from "@/app/store/muxbus-delivery";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { awaitsBrowserAnswer, isAwaitingUser } from "@/app/store/swarm-line";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { parseRuntimeKind, runtimeKindOfBlockMeta } from "@/util/runtime-kind";
import { createMemo, type Accessor } from "solid-js";
import { containerRuntimeNote, containersByAgent, orphanContainers } from "./atlas-containers";
import { isPrFlow, type AtlasFlow } from "./atlas-flow";
import { useAtlasFlows, useCloudPaused } from "./atlas-flow-store";
import {
    buildAtlasGraph,
    type AtlasAgent,
    type AtlasContainer,
    type AtlasGraph,
    type AtlasMachine,
    type AtlasState,
} from "./atlas-graph";
import { prServices } from "./atlas-pr";
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

/** One of this instance's agents, from its Swarm row and its pane's meta, with the container it runs in. */
function localAgent(
    key: string,
    node: AgentTreeNode,
    info: { agentId: string | null; seat: AtlasAgent["seat"] },
    containers: ReadonlyMap<string, AtlasContainer>
): AtlasAgent {
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
    const runtimeId = typeof meta?.["agentId"] === "string" ? (meta["agentId"] as string) : null;
    return {
        key,
        name: node.agentName,
        provider: node.agentProvider,
        state,
        line: node.line.text || null,
        machine: LOCAL_MACHINE,
        agentId: info.agentId,
        route: routeOf(meta),
        seat: info.seat,
        contextTokens: node.contextTokens,
        currentTool: node.currentTool,
        blockId: node.blockId,
        runtimeId,
        remote: false,
        stale: false,
        spend: spendOf(meta),
        runtime: node.blockId ? runtimeKindOfBlockMeta(meta) : null,
        container: (runtimeId && containers.get(runtimeId)) || null,
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
        runtime: parseRuntimeKind(row.runtime) ?? null,
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
        // As its instance reports it; an older build says nothing, and nothing is shown.
        runtime: parseRuntimeKind(agent.runtime) ?? null,
    };
}

const sameItems = <T>(a: readonly T[], b: readonly T[]) => a.length === b.length && a.every((item, i) => item === b[i]);

export interface AtlasData {
    graph: Accessor<AtlasGraph>;
    /** The Swarm's model: its selection and fleet actions are the map's too. */
    model: SwarmViewModel;
    /** The Swarm's list, for its filter bar. */
    swarm: SwarmAgents;
    /** Every agent, by key, for the card. */
    agents: Accessor<Map<string, AtlasAgent>>;
    machines: Accessor<Map<string, AtlasMachine>>;
    /** The messages, task hand-overs and pull request lines on show now, as srv reports them. */
    flows: Accessor<readonly AtlasFlow[]>;
    /** srv's last read of the account's other machines failed: the cloud tier shown may be old, or missing. */
    cloudUnavailable: Accessor<boolean>;
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
            cloud: s.tier === "cloud",
            platform: s.platform,
            stale: s.stale,
            seenAtMs: s.seenAtMs,
        }));
        return [local, ...sections];
    });

    const containerStatus = sharedContainerStatus();
    const agentContainers = createMemo(() => containersByAgent(containerStatus.containers()));

    const agentList = createMemo<AtlasAgent[]>(() => {
        const shown = swarm.filter.shown();
        const info = seats.info();
        const containers = agentContainers();
        const out: AtlasAgent[] = [];
        for (const [key, node] of swarm.keyedRows()) {
            if (shown && !shown.has(key)) continue;
            const i = info.get(key);
            out.push(localAgent(key, node, { agentId: i?.agentId ?? null, seat: i?.seat ?? null }, containers));
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

    // This machine, with what is wrong with its container runtime, if anything.
    const notedMachines = createMemo(() => {
        const note = containerRuntimeNote(containerStatus.runtime(), agentList(), containerStatus.containers());
        return note ? machines().map((m) => (m.key === LOCAL_MACHINE ? { ...m, note } : m)) : machines();
    });
    // A container no agent uses matches no filter: with one on, the map is the agents that match.
    const orphans = createMemo(() => (swarm.filter.shown() ? [] : orphanContainers(containerStatus.containers(), LOCAL_MACHINE)));

    const live = useAtlasFlows();
    const paused = useCloudPaused();
    // The same lines as before is the same list: a message's line coming or going does not rebuild the graph.
    const prLines = createMemo<readonly AtlasFlow[]>(() => live.flows().filter(isPrFlow), undefined, { equals: sameItems });
    const services = createMemo(() => prServices(prLines(), { lastNoticeAtMs: live.lastPrNoticeAtMs(), paused: paused() }, clockTime));

    const graph = createMemo(() => buildAtlasGraph(notedMachines(), agentList(), { orphans: orphans(), services: services() }));
    const agents = createMemo(() => new Map(agentList().map((a) => [a.key, a] as const)));
    const machineMap = createMemo(() => new Map(machines().map((m) => [m.key, m] as const)));
    const cloudUnavailable = () => model.otherInstancesAtom()?.cloud === "unavailable";
    return { graph, model, swarm, agents, machines: machineMap, flows: live.flows, cloudUnavailable };
}
