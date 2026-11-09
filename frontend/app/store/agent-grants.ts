// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agents the person said "Always" to, as srv last said (`agent.notice.grants`, `agent:grants`):
 * today "starts agents without asking" (docs/specs/SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md §4).
 * The card's Always writes one; the agent's pane menu shows it and takes it back. Only the AgentMux
 * window can write one (`agent.notice.setgrant` is operator-only in srv).
 */

import { createSignal } from "solid-js";

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

const [spawnAgents, setSpawnAgents] = createSignal<ReadonlySet<string>>(new Set());

const lower = (agentId: string): string => agentId.toLowerCase();

/** Whether the person let this agent (by definition id) start agents without a notice first. Reactive. */
export function startsAgentsWithoutAsking(agentId: string): boolean {
    return spawnAgents().has(lower(agentId));
}

/** Let the agent start agents without a notice (`allowed`), or take that back. */
export async function setStartsAgentsWithoutAsking(agentId: string, allowed: boolean): Promise<void> {
    const r = await RpcApi.AgentNoticeSetGrantCommand(TabRpcClient, { topic: "spawn", agent_id: agentId, allowed });
    setSpawnAgents(new Set(r.agents.map(lower)));
}

let started = false;

/** Follow srv's grants. Call once per window. */
export function startAgentGrants(): void {
    if (started) return;
    started = true;
    onMuxEvent(WpsEvent.AgentGrants, (e) => {
        if (e.topic === "spawn") setSpawnAgents(new Set(e.agents.map(lower)));
    });
    void RpcApi.AgentNoticeGrantsCommand(TabRpcClient, { topic: "spawn" })
        .then((r) => setSpawnAgents(new Set(r.agents.map(lower))))
        .catch((e) => console.log("agent grants: could not load them", e));
}

/** Tests only. */
export function __setSpawnGrants(agents: string[]): void {
    setSpawnAgents(new Set(agents.map(lower)));
}
