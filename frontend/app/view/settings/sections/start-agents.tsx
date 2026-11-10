// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings → Agents → Starting agents: the global switch for the notice an agent's start of another
// agent shows in its pane (`agent:maystartagents`,
// docs/specs/SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md §4). Off by default. The person flips it
// here; an agent has no call that does.

import type { JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { SettingRow, ToggleControl, set } from "../settings-controls";
import type { SettingsIndexEntry } from "../settings-model";

export const START_AGENTS_SETTING = {
    id: "agents.may_start_agents",
    label: "Agents may start agents",
    description:
        "Off by default. The first time an agent starts another agent, its pane shows a card with a 30 second countdown that goes ahead on its own: Stop it, Start now, or Always allow that agent. Turn this on to skip the card for every agent; its pane still shows a one-line notice.",
    section: "agents",
    group: "Starting agents",
    keywords: ["start agents", "spawn", "workers", "create agent", "countdown", "approval", "autonomy", "agent:maystartagents"],
} satisfies SettingsIndexEntry;

export function StartAgents(): JSX.Element {
    const settings = () => settingsAtom() ?? ({} as Record<string, unknown>);
    return (
        <SettingRow
            id={START_AGENTS_SETTING.id}
            label={START_AGENTS_SETTING.label}
            description={START_AGENTS_SETTING.description}
            control={<ToggleControl checked={settings()["agent:maystartagents"] === true} onChange={(v) => set("agent:maystartagents", v)} />}
        />
    );
}
