// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agents the person said "Always" to, as srv last said (`agent.notice.grants`, `agent:grants`):
 * "starts agents without asking" and "uses Google without asking"
 * (docs/specs/SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md §4). The card's Always writes one; the
 * agent's pane menu shows it and takes it back. Only the AgentMux window can write one
 * (`agent.notice.setgrant` is operator-only in srv).
 */

import { createSignal } from "solid-js";

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

type GrantTopic = "spawn" | "google";
const TOPICS: GrantTopic[] = ["spawn", "google"];

const signals = {
    spawn: createSignal<ReadonlySet<string>>(new Set()),
    google: createSignal<ReadonlySet<string>>(new Set()),
};

const lower = (agentId: string): string => agentId.toLowerCase();

function remember(topic: GrantTopic, agents: string[]): void {
    signals[topic][1](new Set(agents.map(lower)));
}

async function setGrant(topic: GrantTopic, agentId: string, allowed: boolean): Promise<void> {
    const r = await RpcApi.AgentNoticeSetGrantCommand(TabRpcClient, { topic, agent_id: agentId, allowed });
    remember(topic, r.agents);
}

/** Whether the person let this agent (by definition id) start agents without a notice first. Reactive. */
export function startsAgentsWithoutAsking(agentId: string): boolean {
    return signals.spawn[0]().has(lower(agentId));
}

/** Let the agent start agents without a notice (`allowed`), or take that back. */
export function setStartsAgentsWithoutAsking(agentId: string, allowed: boolean): Promise<void> {
    return setGrant("spawn", agentId, allowed);
}

/** Whether the person let this agent use a Google connector without a notice first. Reactive. */
export function usesGoogleWithoutAsking(agentId: string): boolean {
    return signals.google[0]().has(lower(agentId));
}

/** Take back (or give) the agent's Google "Always". */
export function setUsesGoogleWithoutAsking(agentId: string, allowed: boolean): Promise<void> {
    return setGrant("google", agentId, allowed);
}

let started = false;

/** Follow srv's grants. Call once per window. */
export function startAgentGrants(): void {
    if (started) return;
    started = true;
    onMuxEvent(WpsEvent.AgentGrants, (e) => {
        if (e.topic === "spawn" || e.topic === "google") remember(e.topic, e.agents);
    });
    for (const topic of TOPICS) {
        void RpcApi.AgentNoticeGrantsCommand(TabRpcClient, { topic })
            .then((r) => remember(topic, r.agents))
            .catch((e) => console.log("agent grants: could not load them", e));
    }
}

/** Tests only. */
export function __setGrants(topic: GrantTopic, agents: string[]): void {
    remember(topic, agents);
}
