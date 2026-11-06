// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The agent runtime store (agent-runtime.ts) on srv: its list, its per-agent event, `agents:changed`
// and a timer. One store serves the window (the status bar, My Agents and the Swarm read it), so
// srv is asked once and each agent's event is followed once.

import { noteOpenedByPerson } from "@/app/store/agent-open-note";
import { reopenParkedWhereItWas, type ParkedReopenDeps } from "@/app/store/agent-parked";
import { atoms, MOS } from "@/app/store/global";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
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

/** Open an agent: it attaches if it runs without a pane, starts if it is stopped, and focuses the pane that
 *  shows it, in the active tab. The one path My Agents, the Swarm, the picker, the palette and the attention
 *  list's Open take. A parked agent whose workspace still exists is shown there instead, in a window with its
 *  pane focused (store/agent-parked.ts). Resolves to why srv refused, for the caller to say (the agent is open
 *  elsewhere, or its provider can't run here). */
export async function openAgentInActiveTab(
    agent: Pick<AgentRuntime, "agent_id" | "name">
): Promise<string | undefined> {
    // Before the call: the pane can mount before srv answers. The agent opens with its update.
    noteOpenedByPerson(agent.agent_id);
    try {
        const blockId = sharedAgentRuntime()
            .rows()
            .find((r) => r.agent_id === agent.agent_id)?.block_id;
        const open = (tabId: string) =>
            RpcApi.AgentOpenCommand(TabRpcClient, { agent_id: agent.agent_id, tab_id: tabId, focus: true });
        if (await reopenParkedWhereItWas(blockId, parkedDeps(open))) return undefined;
        await open(atoms.activeTabId());
        return undefined;
    } catch (e) {
        console.warn(`[agent-runtime] opening ${agent.name} failed`, e);
        return e instanceof Error ? e.message : String(e);
    }
}

/** What reopening a parked agent reads and does; `open` is agent.open for this agent in the given tab. */
const parkedDeps = (open: (tabId: string) => Promise<unknown>): ParkedReopenDeps => ({
    // A parked agent's block is the one this window is least likely to have: load it rather than read an empty cache.
    blockMeta: async (blockId) => {
        const oref = MOS.makeORef("block", blockId);
        return (MOS.getObjectValue<Block>(oref) ?? (await MOS.reloadMuxObject<Block>(oref)))?.meta;
    },
    tabExists: async (tabId) => {
        const { loadObject } = await import("@/app/util/reveal-block");
        return (await loadObject<Tab>("tab", tabId)) != null;
    },
    // A parked block is in no tab: attaching it to its old tab is what makes it a pane there.
    attach: async (tabId) => {
        await open(tabId);
    },
    // Quiet: the workspace is shown or opened by this, and a failure is not an error here: the pane opens in the active tab.
    reveal: async (blockId) => {
        const { revealBlock } = await import("@/app/util/reveal-block");
        return (await revealBlock(blockId, { source: "open", quiet: true })).ok;
    },
});

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
