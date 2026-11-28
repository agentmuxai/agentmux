// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings → Agents → Browser windows: the switch that makes every browser an agent opens float in a window of its
// own (`agent:floatbrowsers`, docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md §4). Off by default. An
// agent's own `floating` or `split` wins.

import type { JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { SettingRow, ToggleControl, set } from "../settings-controls";
import { settingEntry } from "../settings-model";

export const FLOAT_BROWSERS_SETTING = settingEntry({
    id: "agents.float_browsers",
    labelKey: "settings.agents.floatBrowsers.label",
    descriptionKey: "settings.agents.floatBrowsers.description",
    section: "agents",
    groupKey: "settings.agents.group.browserWindows",
    keywords: ["browser", "floating", "window", "float", "popup", "agent browser", "agent:floatbrowsers"],
});

export function FloatBrowsers(): JSX.Element {
    const settings = () => settingsAtom() ?? ({} as Record<string, unknown>);
    return (
        <SettingRow
            settingKey="agent:floatbrowsers"
            id={FLOAT_BROWSERS_SETTING.id}
            label={FLOAT_BROWSERS_SETTING.label}
            description={FLOAT_BROWSERS_SETTING.description}
            control={<ToggleControl checked={settings()["agent:floatbrowsers"] === true} onChange={(v) => set("agent:floatbrowsers", v)} />}
        />
    );
}
