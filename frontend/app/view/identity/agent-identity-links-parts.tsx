// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent identity links panel's markup in parts (agent-identity-links-panel.tsx): the table of linked accounts,
 * the hint when there are none, the Connect Anthropic account button, the state without an agent, and the Claude
 * login. Each reads the panel's state (agent-identity-links-state.ts).
 */

import { createMemo, For, Show, type JSX } from "solid-js";

import { PROVIDER_LABELS } from "./identity-model";
import { statusBadge } from "./identity-manager";
import type { AgentIdentityRow } from "./agent-identities-model";
import { ProviderLoginPanel } from "@/app/view/accounts/ProviderLoginPanel";
import { canonicalProviderId } from "@/app/view/agent/providers/provider-id-aliases";
import { brandForProvider } from "@/app/view/accounts/provider-brand";
import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { AgentIdentityLinksState } from "./agent-identity-links-state";

type Part = { t: AgentIdentityLinksState };

/** What the panel says when it has no agent to show. */
export function NoAgent(): JSX.Element {
    return (
        <div class="identity-pane-empty">
            <p>{t("agentLinks.noAgent")}</p>
            <p class="identity-pane-empty-hint">
                {t("agentLinks.noAgentHint")}
            </p>
        </div>
    );
}

/** One linked account: its provider, the account, its status, and Connect or Re-login for a Claude row. */
function LinkRow(props: Part & { row: AgentIdentityRow }): JSX.Element {
    const { t: links, row } = props;
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
                          ? PROVIDER_LABELS.anthropic
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
                    (InAppLoginPanel/ProviderLoginPanel) only
                    exists for Claude; other providers still
                    route through the launch dialog. Canonicalize
                    — db_agent_identity_links can carry a
                    legacy-alias row ("claude-code") for a
                    migrated agent (reagent P1 on PR #2414;
                    see provider-id-aliases.ts). */}
                <Show when={canonicalProviderId(row.provider) === "claude"}>
                    <Button
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
                        onClick={() => links.openClaudeLogin(row.accountId, row.provider)}
                    >
                        {row.account ? t("agentLinks.reLogin") : t("agentLinks.connect")}
                    </Button>
                </Show>
            </td>
        </tr>
    );
}

/** The table of this agent's linked accounts, or the hint that there are none (once the links have loaded). */
export function LinkedAccounts(props: Part): JSX.Element {
    const links = props.t;
    return (
        <Show
            when={links.rows().length > 0}
            fallback={
                // reagent P2 on PR #2414 (round 3): gated on
                // linksLoaded() && !error() — without this,
                // this text rendered indistinguishably for
                // "still loading", "load failed" (the panel's error
                // banner already covers that), and a
                // genuinely empty agent.
                <Show when={links.linksLoaded() && !links.error()}>
                    <div class="identity-pane-empty-hint">
                        <p>{t("agentLinks.none")}</p>
                    </div>
                </Show>
            }
        >
            <table class="identity-pane-bindings">
                <thead>
                    <tr>
                        <th>{t("agentLinks.col.provider")}</th>
                        <th>{t("agentLinks.col.account")}</th>
                        <th>{t("agentLinks.col.status")}</th>
                        <th />
                    </tr>
                </thead>
                <tbody>
                    <For each={links.rows()}>{(row) => <LinkRow t={links} row={row} />}</For>
                </tbody>
            </table>
        </Show>
    );
}

/** Connect Anthropic account, for a Claude agent with no Claude link of its own. */
export function ConnectAnthropic(props: Part): JSX.Element {
    const links = props.t;
    // reagent P1 on PR #2414 (round 6): standalone, NOT
    // nested inside LinkedAccounts' no-rows fallback — a
    // Claude-provider agent with other-provider links
    // but no claude/claude-code row of its own needs
    // this same affordance, and the per-row
    // Connect/Re-login button (only rendered on rows
    // whose OWN provider is claude) can't cover it since
    // no such row exists to attach it to. Gated on the
    // agent's own provider (#2414):
    // otherwise this links an unrelated Claude account
    // to an agent whose actual provider is something
    // else entirely.
    return (
        <Show
            when={
                links.linksLoaded() &&
                !links.error() &&
                !links.hasClaudeLink() &&
                canonicalProviderId(links.effectiveAgentProviderId()) === "claude"
            }
        >
            <div class="identity-pane-empty-hint">
                <Button
                    tone="accent"
                    class="identity-btn identity-btn-primary"
                    onClick={() => links.openClaudeLogin(undefined)}
                >
                    {t("agentLinks.connectAnthropic")}
                </Button>
            </div>
        </Show>
    );
}

/** The Claude login, over the panel while it is open. */
export function ClaudeLogin(props: Part & { agentId: string | undefined }): JSX.Element {
    const links = props.t;
    return (
        <Show when={links.claudePanelOpen()}>
            {/* This panel can be hosted inside a pane's ModalLayer
                scope="pane" — "pane" here stacks this dialog correctly
                over that modal instead of both competing for the same
                window-level backdrop. */}
            <ProviderLoginPanel
                onClose={() => links.setClaudePanelOpen(false)}
                existingAccountId={links.claudeRefreshAccountId()}
                linkTarget={props.agentId ? { agentDefinitionId: props.agentId } : undefined}
                staleAliasProvider={links.claudeStaleAliasProvider()}
                scope="pane"
            />
        </Show>
    );
}
