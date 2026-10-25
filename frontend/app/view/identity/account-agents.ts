// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Which agents use an account, with the link rows to remove for each, and the
// unbind itself (docs/specs/SPEC_DROP_THE_AGENT_STASH_2026_10_24.md §3).

import { createSignal } from "solid-js";

import type { AgentDefinition } from "@/app/store/rpc-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { parseAgentAccounts } from "./identity-model";

/** An agent using an account. `providers` are its link rows for the account,
 *  as stored (a legacy alias stays an alias): none means the agent is only in
 *  the old `agent.accounts` index, which has no link to remove. */
export interface AccountAgent {
    id: string;
    name: string;
    providers: string[];
}

// srv announces a removed link only per agent, so every list built from the
// links (the cards' counts, an open card's agents) reloads on this instead.
const [linksRevision, setLinksRevision] = createSignal(0);
export { linksRevision };

/** The agents using `account`: its link rows (what a launch uses) plus the
 *  legacy index, by name, ordered as found. `links` is the all-agents link
 *  list the caller fetched. */
export function agentsUsingAccount(
    accountId: string,
    agents: AgentDefinition[],
    links: { agent_id: string; account_id: string; provider: string }[]
): AccountAgent[] {
    const byId = new Map(agents.map((a) => [a.id, a.name] as const));
    const out = new Map<string, AccountAgent>();
    for (const l of links) {
        if (l.account_id !== accountId) continue;
        const entry = out.get(l.agent_id) ?? {
            id: l.agent_id,
            name: byId.get(l.agent_id) ?? l.agent_id,
            providers: [],
        };
        entry.providers.push(l.provider);
        out.set(l.agent_id, entry);
    }
    for (const a of agents) {
        if (!out.has(a.id) && Object.values(parseAgentAccounts(a)).includes(accountId)) {
            out.set(a.id, { id: a.id, name: a.name, providers: [] });
        }
    }
    return [...out.values()];
}

/** Removes the account from `agent` (every link row it holds for it), then
 *  tells the lists built from the links to reload. Throws if srv refuses. */
export async function unbindAccountFromAgent(agent: AccountAgent): Promise<void> {
    try {
        for (const provider of agent.providers) {
            await RpcApi.UnlinkAgentIdentityCommand(TabRpcClient, { agent_id: agent.id, provider });
        }
    } finally {
        setLinksRevision((n) => n + 1);
    }
}
