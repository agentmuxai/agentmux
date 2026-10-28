// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The agent runtime store (agent-runtime.ts) on srv: its list, its per-agent event, `agents:changed`
// and a timer. One store serves the window (the status bar, My Agents and the Swarm read it), so
// srv is asked once and each agent's event is followed once.

import { atoms } from "@/app/store/global";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { noteOpenedByPerson } from "@/app/view/agent/startup/opening-summary";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createRoot } from "solid-js";
import { createAgentRuntimeStore, type AgentRuntimeStore } from "./agent-runtime";

/** The store on srv. */
export function createLiveAgentRuntimeStore(): AgentRuntimeStore {
    return createAgentRuntimeStore({
        list: () => RpcApi.AgentRuntimeListCommand(TabRpcClient, {}),
        subscribe: (agentId, handler) => onMuxEvent(`agentruntime:changed:${agentId}`, handler),
        onAgentsChanged: (handler) => onMuxEvent(WpsEvent.AgentsChanged, () => handler()),
        every: (ms, tick) => {
            const timer = setInterval(tick, ms);
            return () => clearInterval(timer);
        },
    });
}

let shared: AgentRuntimeStore | undefined;

/** The window's store, made on first use and kept for the life of the window. */
export function sharedAgentRuntime(): AgentRuntimeStore {
    if (!shared) {
        shared = createRoot(() => createLiveAgentRuntimeStore());
        void shared.load();
    }
    return shared;
}

/** Open an agent in the active tab: it attaches if it runs without a pane, starts if it is stopped, and
 *  focuses the pane that shows it. Resolves to why srv refused, for the caller to say (the agent is open
 *  elsewhere, or its provider can't run here). */
export async function openAgentInActiveTab(agent: Pick<AgentRuntime, "agent_id" | "name">): Promise<string | undefined> {
    // Before the call: the pane can mount before srv answers. The agent opens with its update.
    noteOpenedByPerson(agent.agent_id);
    try {
        await RpcApi.AgentOpenCommand(TabRpcClient, {
            agent_id: agent.agent_id,
            tab_id: atoms.activeTabId(),
            focus: true,
        });
        return undefined;
    } catch (e) {
        console.warn(`[agent-runtime] opening ${agent.name} failed`, e);
        return e instanceof Error ? e.message : String(e);
    }
}

/** Start an agent that has no pane, headless: its process runs and its session resumes, and it gets a
 *  row of its own in the Swarm. Resolves to why srv refused (a terminal-tier CLI can't run without a
 *  pane, or the agent can't start now), for the caller to say. */
export async function startAgent(agent: Pick<AgentRuntime, "agent_id" | "name">): Promise<string | undefined> {
    try {
        await RpcApi.AgentStartCommand(TabRpcClient, { agent_id: agent.agent_id });
        return undefined;
    } catch (e) {
        console.warn(`[agent-runtime] starting ${agent.name} failed`, e);
        return e instanceof Error ? e.message : String(e);
    }
}
