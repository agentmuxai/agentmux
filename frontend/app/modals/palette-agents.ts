// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The agents in the command palette, live: the window's runtime rows (the Swarm's and My Agents' source)
 * with each agent's seat from the definitions, and what selecting one does: bring its pane forward (the
 * tab or window it is in), or open it as the Swarm's Open does.
 */

import { getOpenDefinitionMap } from "@/app/store/agent-pane-state-store";
import { openAgentInActiveTab, sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import type { CommandEntry } from "@/app/store/command-registry";
import { pushNotification } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { windowId } from "@/app/store/window-identity";
import { mergeOpenDefinitions } from "@/app/view/agent/open-agent-panes";
import { useAgentDefinitions } from "@/app/view/agent/components/agent-definition-hooks";
import { revealBlock } from "@/app/util/reveal-block";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createMemo } from "solid-js";
import { agentEntries } from "./palette-agent-entries";

/** Where the agent is open, anywhere in this AgentMux: this window's panes, and srv's for the others. */
async function openBlockOf(agentId: string): Promise<string | undefined> {
    let panes: Awaited<ReturnType<typeof RpcApi.AgentOpenPanesCommand>>["panes"] = [];
    try {
        panes = (await RpcApi.AgentOpenPanesCommand(TabRpcClient, {}))?.panes ?? [];
    } catch {
        // srv could not say: this window's panes are all there is to go on.
    }
    return mergeOpenDefinitions(getOpenDefinitionMap(), panes, windowId()).get(agentId);
}

/** Selecting an agent: show its pane, or open it when it has none. */
export async function goToAgent(agent: Pick<AgentRuntime, "agent_id" | "name">): Promise<void> {
    const blockId = await openBlockOf(agent.agent_id);
    if (blockId) {
        await revealBlock(blockId, { source: "palette" });
        return;
    }
    const why = await openAgentInActiveTab(agent);
    if (why) {
        pushNotification({
            icon: "fa-triangle-exclamation",
            title: `Couldn't open ${agent.name}`,
            message: why,
            timestamp: new Date().toISOString(),
            type: "error",
            expiration: Date.now() + 8000,
        });
    }
}

/** The palette's agent entries, following the runtime rows and the definitions. Call inside a component. */
export function usePaletteAgents(): () => CommandEntry[] {
    const rows = sharedAgentRuntime().rows;
    const [definitions] = useAgentDefinitions();
    const seats = createMemo(() => new Map(definitions().map((d) => [d.id, d.seat] as const)));
    return createMemo(() => agentEntries(rows(), seats(), goToAgent));
}
