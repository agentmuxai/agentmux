// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// AgentIdentityLinksPanel — read-only Provider/Account/Status table for
// ONE agent's direct account links (`db_agent_identity_links`, the table
// the spawn-time resolver actually reads —
// crates/srv/src/identity/resolver/inject.rs). Given an `agentId`, this is the
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
// (or never had one) had no in-app path to fix it short of the launch
// dialog's own "New Agent" flow, which doesn't apply to an agent that
// already exists. New agent
// identities for OTHER providers, and non-Claude relogin, remain out of
// scope — created from the agent-launch flow directly, per
// docs/specs/SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC_2026_07_10.md.

import { Show, type JSX } from "solid-js";

import { createAgentIdentityLinks, type AgentIdentityLinksPanelProps } from "./agent-identity-links-state";
import { ClaudeLogin, ConnectAnthropic, LinkedAccounts, NoAgent } from "./agent-identity-links-parts";

import "./identity-pane-view.scss";

export const AgentIdentityLinksPanel = (props: AgentIdentityLinksPanelProps): JSX.Element => {
    const t = createAgentIdentityLinks(props);
    return (
        <div class="identity-pane">
            <div class="identity-pane-detail">
                <Show when={t.error()}>
                    <div class="identity-pane-error">{t.error()}</div>
                </Show>

                <Show when={props.agentId} fallback={<NoAgent />}>
                    <div class="identity-pane-readonly">
                        <h2 class="identity-pane-name">{t.agent()?.name ?? "This agent"}</h2>

                        <h3 class="identity-pane-section-title">Linked accounts</h3>
                        <LinkedAccounts t={t} />
                        <ConnectAnthropic t={t} />
                    </div>
                </Show>
            </div>

            <ClaudeLogin t={t} agentId={props.agentId} />
        </div>
    );
};

AgentIdentityLinksPanel.displayName = "AgentIdentityLinksPanel";
