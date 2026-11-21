// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent as a person reads it. srv records an agent by its id ("tess-2": the browser pane's owner, a bypass's
 * agent, the agent a pane can be given back to), and what a person is shown is its name ("Tess"). One list of the
 * agents, kept current while a view that names one is on screen, and one lookup for it.
 */

import { createDomainStore } from "@/app/store/domain-store";
import { RpcApi, type AgentDefinition } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

const store = createDomainStore<AgentDefinition[]>({
    initial: [],
    events: ["AgentsChanged"],
    load: () => RpcApi.ListAgentDefinitionsCommand(TabRpcClient, { is_seeded: 0 }),
});

/** In a component that shows an agent's name: the list of agents is loaded, and kept current while it is mounted. */
export function followAgentNames(): void {
    store.follow();
}

/** The name of the agent `id` is in `agents`: its name, else the id as it was given (no agent has it, or the list is not loaded). */
export function nameInAgents(agents: Pick<AgentDefinition, "id" | "slug" | "name">[], id: string): string {
    const key = id.trim().toLowerCase();
    const found = key ? agents.find((a) => a.slug?.toLowerCase() === key || a.id?.toLowerCase() === key) : undefined;
    return found?.name?.trim() || id;
}

/** Every agent in the list, for what recognises a name in text. Reactive: it follows the list of agents. */
export function agentRoster(): Pick<AgentDefinition, "id" | "slug" | "name">[] {
    return store.data();
}

/** An agent as a person reads it, for the id srv records it by. Reactive: it follows the list of agents. */
export function agentDisplayName(id: string): string {
    return nameInAgents(store.data(), id);
}
