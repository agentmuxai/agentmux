// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm Supervisor's settings in the Settings pane's search: one entry, because they live in the
// Swarm's Supervisor section (the switch, the limits, the wait, the fleet budget, the notices), not
// here. Choosing it opens that section's settings panel.
// docs/specs/SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §6.2.

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { requestSupervisorSettingsOpen } from "@/app/store/supervisor-settings-request";
import type { SettingsIndexEntry } from "../settings-model";

/** Show the Supervisor's settings: the tab's Swarm on its Supervisor section, or a new one, panel open. */
export async function openSupervisorSettings(): Promise<void> {
    const blockId = await openOrFocusPaneByView(
        "swarm",
        { meta: { view: "swarm", "swarm:section": "supervisor" } },
        { "swarm:section": "supervisor" }
    );
    // Only the pane that was focused or created answers, not a Swarm in another tab.
    if (blockId) requestSupervisorSettingsOpen(blockId);
}

export const SUPERVISOR_SETTINGS = {
    supervisor: {
        id: "agents.supervisor",
        label: "Swarm Supervisor",
        description: "Opens the Swarm's Supervisor settings: the switch, limits, wait before continuing, fleet budget and notices",
        section: "agents",
        keywords: [
            "supervisor",
            "steady",
            "drive",
            "continue",
            "continuation",
            "keep going",
            "autopilot",
            "cooldown",
            "wait before continuing",
            "fleet budget",
            "continuations per hour",
            "no progress",
            "swarm",
            "supervisor:enabled",
            "supervisor:cooldown",
            "supervisor:hourly",
            "supervisor:steady:continuations",
            "supervisor:drive:continuations",
            "supervisor:notify",
        ],
        opens: openSupervisorSettings,
    },
} satisfies Record<string, SettingsIndexEntry>;
