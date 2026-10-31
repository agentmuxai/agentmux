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
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { mergeOpenDefinitions } from "@/app/view/agent/open-agent-panes";
import { revealBlock } from "@/app/util/reveal-block";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import type { AgentSeat } from "@/types/rpc/AgentSeat";
import { createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { agentEntries } from "./palette-agent-entries";

/** Where the agent is open, anywhere in this AgentMux: this window's panes, and srv's for the others. */
async function openBlockOf(agentId: string): Promise<string | undefined> {
    let panes: Awaited<ReturnType<typeof RpcApi.AgentOpenPanesCommand>>["panes"] = [];
    try {
        panes = (await RpcApi.AgentOpenPanesCommand(TabRpcClient, {}))?.panes ?? [];
    } catch {
        // srv could not say: this window's panes are all there is to go on.
    }
    // Loaded here: the module reads the object store as it is evaluated, which a palette imported
    // everywhere (the command registry, the menus) must not do.
    const { windowId } = await import("@/app/store/window-identity");
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

/** Each agent's seat by id, read when the palette opens and again when the agents change. */
function useAgentSeats(): () => ReadonlyMap<string, AgentSeat | undefined> {
    const [seats, setSeats] = createSignal<ReadonlyMap<string, AgentSeat | undefined>>(new Map());
    onMount(() => {
        let cancelled = false;
        const load = async () => {
            try {
                const defs = await RpcApi.ListAgentDefinitionsCommand(TabRpcClient);
                if (!cancelled) setSeats(new Map((defs ?? []).map((d) => [d.id, d.seat] as const)));
            } catch {
                // No seats until srv answers: the agents are listed without them.
            }
        };
        void load();
        const unsub = onMuxEvent(WpsEvent.AgentsChanged, () => void load());
        onCleanup(() => {
            cancelled = true;
            unsub();
        });
    });
    return seats;
}

/** The palette's agent entries, following the runtime rows and the definitions. Call inside a component. */
export function usePaletteAgents(): () => CommandEntry[] {
    const rows = sharedAgentRuntime().rows;
    const seats = useAgentSeats();
    return createMemo(() => agentEntries(rows(), seats(), goToAgent));
}
