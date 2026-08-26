// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// AgentIdentityLinksPanel — read-only Provider/Account/Status table for
// ONE agent's direct account links (`db_agent_identity_links`, the table
// the spawn-time resolver actually reads —
// agentmux-srv/src/identity/resolver.rs). Given an `agentId`, this is the
// same row shape the now-removed Armory "Identities" rail tab
// (`AgentIdentitiesPanel`) rendered per selected agent — extracted here so
// the agent-pane's own `view: "identity"` tab (`identity-pane-view.tsx`)
// can render it directly for its own block's agent, closing the data gap
// documented in docs/specs/SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md
// §1.3.
//
// Create/edit/delete/bind/unbind stayed out of scope here EXCEPT for one
// exception carved out by docs/specs/SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md
// §3.3 surface 3: a Connect/Re-login action for Claude specifically, since
// this is otherwise the only place a broken/missing Claude binding is
// visible per-agent — without it, an agent whose bound account was deleted
// (or, per retro-agentu-0.54.9-stuck-error-2026-08-03.md, never had one) had
// no in-app path to fix it short of the launch dialog's own "New Agent"
// flow, which doesn't apply to an agent that already exists. New agent
// identities for OTHER providers, and non-Claude relogin, remain out of
// scope — created from the agent-launch flow directly, per
// docs/specs/SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC_2026_07_10.md.

import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show, type JSX } from "solid-js";

import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import { resolveEffectiveLaunchProvider } from "@/app/view/agent/agent-launch-env";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { loadAccounts, subscribeAccountChanges, PROVIDER_LABELS, type Account } from "./identity-model";
import { statusBadge } from "./identity-manager";
import { joinAgentIdentityRows } from "./agent-identities-model";
import { ClaudeLoginPanel } from "@/app/view/accounts/ClaudeLoginPanel";
import { canonicalProviderId } from "@/app/view/agent/providers/provider-id-aliases";
import { brandForProvider } from "@/app/view/accounts/provider-brand";

import "./identity-pane-view.scss";

interface AgentIdentityLinksPanelProps {
    /** The agent whose linked accounts to show. `undefined` when this
     *  panel has no agent context (e.g. a generically opened identity
     *  block) — renders a context-free empty state in that case. */
    agentId: string | undefined;
}

export const AgentIdentityLinksPanel = (props: AgentIdentityLinksPanelProps): JSX.Element => {
    const [agents] = useAgentDefinitions();

    const [allLinks, setAllLinks] = createSignal<AgentDefinitionIdentity[]>([]);
    const [accounts, setAccounts] = createSignal<Account[]>(loadAccounts());
    const [error, setError] = createSignal<string | null>(null);
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
    // reagent P2 on PR #2414 (round 3): the empty-state "Connect Claude
    // account" button below must not appear before the initial load
    // resolves (was previously indistinguishable from "genuinely zero
    // links") or after it fails (the top error banner already covers that
    // case — offering the button too just invites clicking it against
    // still-broken/unknown state).
    const [linksLoaded, setLinksLoaded] = createSignal(false);

    const openClaudeLogin = (existingAccountId: string | undefined, rawProvider?: string) => {
        setClaudeRefreshAccountId(existingAccountId);
        setClaudeStaleAliasProvider(
            rawProvider && rawProvider !== "claude" ? rawProvider : undefined,
        );
        setClaudePanelOpen(true);
    };

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

    const unsubAccounts = subscribeAccountChanges(setAccounts);
    onCleanup(unsubAccounts);

    // Re-subscribe to this agent's own link-change event whenever the
    // agentId prop changes — mirrors AgentLaunchModal.tsx's
    // `identitybundlebindings:changed:<id>` pattern.
    createEffect(() => {
        const id = props.agentId;
        if (!id) return;
        const unsub = waveEventSubscribe({
            eventType: `agentidentities:changed:${id}`,
            handler: () => void refreshLinks(),
        });
        onCleanup(unsub);
    });

    const accountsById = createMemo<Map<string, Account>>(() => {
        const m = new Map<string, Account>();
        for (const a of accounts()) m.set(a.id, a);
        return m;
    });

    const agent = createMemo<AgentDefinition | null>(() => {
        const id = props.agentId;
        if (!id) return null;
        return agents().find((a) => a.id === id) ?? null;
    });

    // Resolve through the agent's bound bundle rather than the possibly-
    // drifted `agent.provider` column directly — #2594, same "gate vs.
    // actual launch can disagree" risk class #2592/#2596/#2607/#2609/
    // #2610 fixed. Gates the "Connect Claude account" CTA below; falls
    // back to `agent()?.provider` while loading/unbound/on failure, same
    // fallback contract `resolveEffectiveLaunchProvider` itself documents
    // — a brief stale flash here is cosmetic (button visibility only),
    // not a spawn/credential decision.
    const [resolvedAgentProviderId] = createResource(agent, (a) =>
        a ? resolveEffectiveLaunchProvider(a) : Promise.resolve(""),
    );
    const effectiveAgentProviderId = () => resolvedAgentProviderId() || (agent()?.provider ?? "");

    const rows = createMemo(() => {
        const id = props.agentId;
        if (!id) return [];
        return joinAgentIdentityRows(id, allLinks(), accountsById());
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

    return (
        <div class="identity-pane">
            <div class="identity-pane-detail">
                <Show when={error()}>
                    <div class="identity-pane-error">{error()}</div>
                </Show>

                <Show
                    when={props.agentId}
                    fallback={
                        <div class="identity-pane-empty">
                            <p>This pane isn't attached to a specific agent.</p>
                            <p class="identity-pane-empty-hint">
                                Open the Identity tab from an agent's own pane to see the accounts
                                that agent launches with.
                            </p>
                        </div>
                    }
                >
                    <div class="identity-pane-readonly">
                        <h2 class="identity-pane-name">{agent()?.name ?? "This agent"}</h2>

                        <h3 class="identity-pane-section-title">Linked accounts</h3>
                        <Show
                            when={rows().length > 0}
                            fallback={
                                // reagent P2 on PR #2414 (round 3): gated on
                                // linksLoaded() && !error() — without this,
                                // this text rendered indistinguishably for
                                // "still loading", "load failed" (the banner
                                // above already covers that), and a
                                // genuinely empty agent.
                                <Show when={linksLoaded() && !error()}>
                                    <div class="identity-pane-empty-hint">
                                        <p>This agent has no linked accounts yet. Link one from its launch dialog.</p>
                                    </div>
                                </Show>
                            }
                        >
                            <table class="identity-pane-bindings">
                                <thead>
                                    <tr>
                                        <th>Provider</th>
                                        <th>Account</th>
                                        <th>Status</th>
                                        <th />
                                    </tr>
                                </thead>
                                <tbody>
                                    <For each={rows()}>
                                        {(row) => {
                                            const badge = createMemo(() => statusBadge(row.account?.status));
                                            return (
                                                <tr>
                                                    <td>
                                                        {(PROVIDER_LABELS as Record<string, string>)[
                                                            brandForProvider(row.provider)
                                                        ] ?? row.provider}
                                                    </td>
                                                    <td>
                                                        {row.account
                                                            ? row.account.display_name?.trim() ||
                                                              // Anthropic accounts persist an
                                                              // internal `${provider}-oauth` name
                                                              // (identity_auth_persist.rs) with no
                                                              // display_name set — fall back to the
                                                              // brand label instead of leaking that
                                                              // internal name (codex P2 on PR #2806).
                                                              (brandForProvider(row.provider) === "anthropic"
                                                                  ? "Anthropic"
                                                                  : row.account.name)
                                                            : "—"}
                                                    </td>
                                                    <td>
                                                        <span class="identity-pane-status">
                                                            <span
                                                                class={`identity-pane-status-dot is-${badge().dot}`}
                                                                aria-hidden="true"
                                                            />
                                                            <span class="identity-pane-status-label">
                                                                {badge().label}
                                                            </span>
                                                        </span>
                                                    </td>
                                                    <td>
                                                        {/* Claude-only for now — the in-app session
                                                            (InAppLoginPanel/ClaudeLoginPanel) only
                                                            exists for Claude; other providers still
                                                            route through the launch dialog. Canonicalize
                                                            — db_agent_identity_links can carry a
                                                            legacy-alias row ("claude-code") for a
                                                            migrated agent (reagent P1 on PR #2414;
                                                            see provider-id-aliases.ts). */}
                                                        <Show when={canonicalProviderId(row.provider) === "claude"}>
                                                            <button
                                                                type="button"
                                                                class="identity-btn identity-btn-secondary"
                                                                // reagent P2 on PR #2414 (round 5): row.accountId
                                                                // (from the link row itself, agent-identities-model.ts)
                                                                // is always present when a link exists; row.account
                                                                // (the JOINED Account object) is null whenever the
                                                                // local accounts cache hasn't caught up yet — using
                                                                // row.account?.id here passed undefined on a
                                                                // stale-cache click, minting a brand-new Claude
                                                                // account instead of refreshing the one the link
                                                                // row already points at.
                                                                onClick={() => openClaudeLogin(row.accountId, row.provider)}
                                                            >
                                                                {row.account ? "Re-login" : "Connect"}
                                                            </button>
                                                        </Show>
                                                    </td>
                                                </tr>
                                            );
                                        }}
                                    </For>
                                </tbody>
                            </table>
                        </Show>

                        {/* reagent P1 on PR #2414 (round 6): standalone, NOT
                            nested inside the rows()===0 fallback above — a
                            Claude-provider agent with other-provider links
                            but no claude/claude-code row of its own needs
                            this same affordance, and the per-row
                            Connect/Re-login button (only rendered on rows
                            whose OWN provider is claude) can't cover it since
                            no such row exists to attach it to. The
                            retro-agentu-0.54.9-stuck-error-2026-08-03.md case
                            this button exists for. Gated on the agent's own
                            provider (reagent P2 on PR #2414, round 3):
                            otherwise this links an unrelated Claude account
                            to an agent whose actual provider is something
                            else entirely. */}
                        <Show
                            when={
                                linksLoaded() &&
                                !error() &&
                                !hasClaudeLink() &&
                                canonicalProviderId(effectiveAgentProviderId()) === "claude"
                            }
                        >
                            <div class="identity-pane-empty-hint">
                                <button
                                    type="button"
                                    class="identity-btn identity-btn-primary"
                                    onClick={() => openClaudeLogin(undefined)}
                                >
                                    Connect Anthropic account
                                </button>
                            </div>
                        </Show>
                    </div>
                </Show>
            </div>

            <Show when={claudePanelOpen()}>
                {/* AgentStashModal (this panel's host) is itself inside
                    agent-view.tsx's ModalLayer scope="pane" — "pane" here
                    stacks this dialog correctly over the Stash modal instead
                    of both competing for the same window-level backdrop. */}
                <ClaudeLoginPanel
                    onClose={() => setClaudePanelOpen(false)}
                    existingAccountId={claudeRefreshAccountId()}
                    linkTarget={props.agentId ? { agentDefinitionId: props.agentId } : undefined}
                    staleAliasProvider={claudeStaleAliasProvider()}
                    scope="pane"
                />
            </Show>
        </div>
    );
};

AgentIdentityLinksPanel.displayName = "AgentIdentityLinksPanel";
