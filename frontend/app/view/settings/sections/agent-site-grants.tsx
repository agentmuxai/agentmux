// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Agents on sites: the standing list of what an agent may do on a site
// without asking each time (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §3).
// A row names one agent and one site and the powers it carries; Revoke ends it
// at once, and a call in flight stops. Only this window can write one
// (crates/srv/src/server/app_api/browser_grants.rs), and an agent's own
// request for auto-approve can add one when the person answers "Always".

import { Notice } from "@/app/element/ui";
import { Show, type JSX } from "solid-js";

import { settingEntry, type SettingsIndexEntry } from "../settings-model";
import { LATE_PROMPT_LIMIT } from "./agent-site-grants-model";
import { GrantsBody } from "./agent-site-grants-parts";
import { createAgentSiteGrants } from "./agent-site-grants-state";

export const AGENT_SITE_GRANTS_SETTING = settingEntry({
    id: "browser.agent_sites",
    labelKey: "settings.browser.agentSites.label",
    descriptionKey: "settings.browser.agentSites.description",
    section: "browser",
    keywords: ["agent", "agents", "site", "sites", "grant", "auto-approve", "terminal", "cloud shell", "revoke", "always", "standing", "permission"],
});

export function AgentSiteGrants(): JSX.Element {
    const s = createAgentSiteGrants();
    return (
        <div id={`setting-${AGENT_SITE_GRANTS_SETTING.id}`} class="setting-row agent-site-grants">
            <div class="setting-devices-description">{AGENT_SITE_GRANTS_SETTING.description}</div>
            <Notice tone="warning" title={<>One limit of the Terminal power</>}>
                {LATE_PROMPT_LIMIT}
            </Notice>
            <Show when={s.error()}>
                <div class="settings-config-error" role="alert">
                    {s.error()}
                </div>
            </Show>
            <Show
                when={s.data()}
                fallback={
                    <Show when={!s.error()}>
                        <div class="setting-devices-empty">Loading…</div>
                    </Show>
                }
            >
                {(d) => <GrantsBody s={s} d={d} />}
            </Show>
        </div>
    );
}
