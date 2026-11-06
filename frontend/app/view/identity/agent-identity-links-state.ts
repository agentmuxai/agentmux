// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent identity links panel's state (agent-identity-links-panel.tsx): this agent's links as the join of what
 * srv has and the account cache, the provider the agent really runs, and the Claude Connect/Re-login panel. The
 * pieces are made in the order the panel always made them (the read of the links, the account subscription, the
 * link-change subscription, the memos, the provider lookup).
 */

import { createEffect, createMemo, createResource, createSignal, onCleanup } from "solid-js";

import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import { resolveEffectiveLaunchProvider } from "@/app/view/agent/agent-launch-env";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { onMuxEvent } from "@/app/store/mps";
import { loadAccounts, subscribeAccountChanges, type Account } from "./identity-model";
import { joinAgentIdentityRows } from "./agent-identities-model";
import { canonicalProviderId } from "@/app/view/agent/providers/provider-id-aliases";

import type { AgentDefinition, AgentDefinitionIdentity } from "@/app/store/rpc-api";

export interface AgentIdentityLinksPanelProps {
    /** The agent whose linked accounts to show. `undefined` when this
     *  panel has no agent context (e.g. a generically opened identity
     *  block) — renders a context-free empty state in that case. */
    agentId: string | undefined;
}

/** All agents' links as srv has them; read once now, and again when told. */
function createLinks() {
    const [allLinks, setAllLinks] = createSignal<AgentDefinitionIdentity[]>([]);
    const [error, setError] = createSignal<string | null>(null);
    // The "Connect Anthropic account" button (ConnectAnthropic,
    // agent-identity-links-parts.tsx) must not appear before the initial load
    // resolves (indistinguishable from "genuinely zero links") or after it
    // fails (the panel's error banner covers that case, and the button would
    // invite a click against still-broken/unknown state) (#2414).
    const [linksLoaded, setLinksLoaded] = createSignal(false);

    const refreshLinks = async (): Promise<void> => {
        try {
            const result = await RpcApi.ListAllAgentIdentitiesCommand(TabRpcClient);
            setAllLinks(result ?? []);
            setError(null);
        } catch (e: any) {
            setError(e?.message ?? "Failed to load agent identities");
        } finally {
            setLinksLoaded(true);
        }
    };
    void refreshLinks();
    return { allLinks, error, linksLoaded, refreshLinks };
}

/** The account cache, kept current while the panel is mounted. */
function createAccounts() {
    const [accounts, setAccounts] = createSignal<Account[]>(loadAccounts());
    const unsubAccounts = subscribeAccountChanges(setAccounts);
    onCleanup(unsubAccounts);
    return accounts;
}

/** The Claude Connect/Re-login panel: whether it is open, for which account, and which legacy alias to clean up. */
function createClaudeLogin() {
    // Set to the account id being refreshed (or "" for a fresh connect —
    // distinguished from "closed" by claudePanelOpen()) when the Claude
    // Connect/Re-login panel is open.
    const [claudePanelOpen, setClaudePanelOpen] = createSignal(false);
    const [claudeRefreshAccountId, setClaudeRefreshAccountId] = createSignal<string | undefined>(undefined);
    // reagent P0 on PR #2414: see ClaudeLoginPanel's staleAliasProvider doc
    // comment — set when the row being acted on carries a legacy-alias
    // provider string, so the panel can unlink it after a successful login
    // instead of leaving an orphaned row that silently blocks every future
    // spawn under the new canonical link.
    const [claudeStaleAliasProvider, setClaudeStaleAliasProvider] = createSignal<string | undefined>(undefined);

    const openClaudeLogin = (existingAccountId: string | undefined, rawProvider?: string) => {
        setClaudeRefreshAccountId(existingAccountId);
        setClaudeStaleAliasProvider(
            rawProvider && rawProvider !== "claude" ? rawProvider : undefined,
        );
        setClaudePanelOpen(true);
    };
    return { claudePanelOpen, setClaudePanelOpen, claudeRefreshAccountId, claudeStaleAliasProvider, openClaudeLogin };
}

/** The agent shown, and the provider it really runs. */
function createAgentProvider(props: AgentIdentityLinksPanelProps, agents: () => AgentDefinition[]) {
    const agent = createMemo<AgentDefinition | null>(() => {
        const id = props.agentId;
        if (!id) return null;
        return agents().find((a) => a.id === id) ?? null;
    });

    // Same resolver as the launch and the credential gate (#2594), so the
    // "Connect Anthropic account" CTA (ConnectAnthropic) follows the provider
    // the agent actually runs; falls
    // back to `agent()?.provider` while loading/unbound/on failure, same
    // fallback contract `resolveEffectiveLaunchProvider` itself documents
    // — a brief stale flash here is cosmetic (button visibility only),
    // not a spawn/credential decision.
    const [resolvedAgentProviderId] = createResource(agent, (a) =>
        a ? resolveEffectiveLaunchProvider(a) : Promise.resolve(""),
    );
    const effectiveAgentProviderId = () => resolvedAgentProviderId() || (agent()?.provider ?? "");
    return { agent, effectiveAgentProviderId };
}

/** The agent identity links panel's state: everything its markup reads, and what its controls do. */
export function createAgentIdentityLinks(props: AgentIdentityLinksPanelProps) {
    const [agents] = useAgentDefinitions();
    const links = createLinks();
    const accounts = createAccounts();

    // Re-subscribe to this agent's own link-change event whenever the
    // agentId prop changes.
    createEffect(() => {
        const id = props.agentId;
        if (!id) return;
        const unsub = onMuxEvent(`agentidentities:changed:${id}`, () => void links.refreshLinks());
        onCleanup(unsub);
    });

    const accountsById = createMemo<Map<string, Account>>(() => {
        const m = new Map<string, Account>();
        for (const a of accounts()) m.set(a.id, a);
        return m;
    });
    const provider = createAgentProvider(props, agents);
    const claude = createClaudeLogin();

    const rows = createMemo(() => {
        const id = props.agentId;
        if (!id) return [];
        return joinAgentIdentityRows(id, links.allLinks(), accountsById());
    });

    // reagent P1 on PR #2414 (round 6): the empty-state Connect button was
    // gated on `rows().length === 0`, so a Claude-provider agent with ANY
    // other-provider link (e.g. a github row, but no claude/claude-code
    // row) fell into the table branch instead — where the per-row
    // Connect/Re-login button only renders on rows whose OWN provider is
    // claude, so there was no row to attach it to either. Net effect: no
    // in-app affordance anywhere in the panel to connect Claude for that
    // agent, despite this being exactly the broken/missing-Claude-binding
    // case the panel exists to fix. Computed independently of rows().length
    // so it applies whether the table renders or not.
    const hasClaudeLink = createMemo(() => rows().some((r) => canonicalProviderId(r.provider) === "claude"));

    return { ...links, ...provider, ...claude, rows, hasClaudeLink };
}

/** What the agent identity links panel's markup reads. */
export type AgentIdentityLinksState = ReturnType<typeof createAgentIdentityLinks>;
