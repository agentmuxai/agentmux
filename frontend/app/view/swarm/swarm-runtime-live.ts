// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The agent runtime store (swarm-runtime.ts) on srv: its list, its per-agent event, `agents:changed`
// and a timer; and the Open of a row.

import { atoms } from "@/app/store/global";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createAgentRuntimeStore, type AgentRuntimeStore } from "./swarm-runtime";

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

/** Open an agent that has no pane, in the active tab (it starts, as opening one does). */
export async function openStoppedAgent(row: AgentRuntime): Promise<void> {
    try {
        await RpcApi.AgentOpenCommand(TabRpcClient, {
            agent_id: row.agent_id,
            tab_id: atoms.activeTabId(),
            focus: true,
        });
    } catch (e) {
        console.warn(`[swarm] opening ${row.name} failed`, e);
    }
}
