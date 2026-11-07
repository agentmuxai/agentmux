// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Agents on sites, the state: the grants and agents as srv lists them, the add form, and the one-change-then-
// relist call every button goes through.

import { createSignal } from "solid-js";

import { createDomainStore } from "@/app/store/domain-store";
import { RpcApi, type AgentDefinition } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { sortByLastUsed } from "@/app/view/agent/agent-recency";
import type { BrowserGrantsResult } from "@/types/rpc/BrowserGrantsResult";

export type TerminalChoice = "none" | "terminal" | "terminal_ask";

export type AgentSiteGrantsState = ReturnType<typeof createAgentSiteGrants>;

const agentName = (a: AgentDefinition) => a.name || a.slug || a.id;

type GrantsData = { grants: BrowserGrantsResult; agents: AgentDefinition[] };

// The rows change here, and also when the person answers "Always" on an agent's request banner, which
// srv announces as `browsergrants:changed`; the agent names come with `agents:changed`.
const store = createDomainStore<GrantsData | null>({
    initial: null,
    events: ["BrowserGrantsChanged", "AgentsChanged"],
    load: async () => {
        const [grants, agents] = await Promise.all([
            RpcApi.BrowserGrantsListCommand(TabRpcClient),
            RpcApi.ListAgentDefinitionsCommand(TabRpcClient, { is_seeded: 0 }),
        ]);
        return { grants, agents };
    },
});

export function createAgentSiteGrants() {
    store.follow();
    const data = () => store.data()?.grants ?? null;
    const agents = () => store.data()?.agents ?? [];
    const [changeError, setChangeError] = createSignal<string | null>(null);
    const error = () => changeError() ?? (store.error() && `Couldn't load the agents on sites: ${store.error()}`);
    const [busy, setBusy] = createSignal(false);
    const [pick, setPick] = createSignal("");
    const [site, setSite] = createSignal("");
    const [terminal, setTerminal] = createSignal<TerminalChoice>("none");
    const now = () => Math.floor(Date.now() / 1000);

    /** One change, then the list as srv now has it. */
    const run = async (what: string, change: () => Promise<BrowserGrantsResult>) => {
        setBusy(true);
        try {
            const grants = await change();
            const current = store.data();
            if (current) store.set({ ...current, grants });
            setChangeError(null);
            return true;
        } catch (e) {
            setChangeError(`Couldn't ${what}: ${e instanceof Error ? e.message : e}`);
            return false;
        } finally {
            setBusy(false);
        }
    };

    const nameOfSlug = (slug: string) => {
        const a = agents().find((d) => d.slug.toLowerCase() === slug.toLowerCase());
        return a ? agentName(a) : slug;
    };
    const candidates = () => sortByLastUsed(agents(), agentName);
    const picked = () => candidates().find((a) => a.slug === pick());

    const add = async () => {
        const agent = picked();
        if (!agent || !site().trim()) return;
        const ok = await run(`let ${agentName(agent)} onto ${site().trim()}`, () =>
            RpcApi.BrowserGrantsSetCommand(TabRpcClient, {
                agent: agent.slug,
                site: site().trim(),
                powers: terminal() === "none" ? [] : [terminal()],
            })
        );
        if (ok) setSite("");
    };

    return {
        data,
        error,
        busy,
        pick,
        setPick,
        site,
        setSite,
        terminal,
        setTerminal,
        now,
        run,
        nameOfSlug,
        candidates,
        picked,
        add,
        agentName,
    };
}
