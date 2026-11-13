// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Agents: what concerns the agents themselves rather than a pane: whether they may
// drive desktop apps (desktop-driving.tsx), whether starting agents shows a card (start-agents.tsx), what an agent pane shows by default (agent-view-setting.tsx), and their ids: forks whose id an older
// build chained, and updating them (fork-slug-migration.tsx).

import { t } from "@/app/i18n/i18n";
import type { JSX } from "solid-js";

import { SectionHeader } from "../settings-controls";
import { AGENT_VIEW_SETTING, AgentViewSetting } from "./agent-view-setting";
import { DESKTOP_DRIVING_SETTING, DesktopDriving } from "./desktop-driving";
import { FORK_SLUG_SETTING, ForkSlugMigration } from "./fork-slug-migration";
import { START_AGENTS_SETTING, StartAgents } from "./start-agents";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const AGENTS_SETTINGS = {
    desktopDriving: DESKTOP_DRIVING_SETTING,
    startAgents: START_AGENTS_SETTING,
    agentView: AGENT_VIEW_SETTING,
    outdatedIds: FORK_SLUG_SETTING,
};

export function AgentsSection(): JSX.Element {
    return (
        <div class="settings-section-body">
            <SectionHeader label={t("settings.agents.group.desktopDriving")} />
            <DesktopDriving />
            <SectionHeader label={t("settings.agents.group.startingAgents")} />
            <StartAgents />
            <SectionHeader label={t("settings.agents.group.agentView")} />
            <AgentViewSetting />
            <SectionHeader label={t("settings.agents.group.agentIds")} />
            <ForkSlugMigration />
        </div>
    );
}
