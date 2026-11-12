// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings → Agents → Desktop driving: the global switch for the `Desktop` tool
// (`desktop:enabled`, docs/specs/SPEC_DESKTOP_DRIVING_2026_10_20.md §5.7). Off by
// default. The person flips it here; an agent has no call that does.

import type { JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { SettingRow, ToggleControl, set } from "../settings-controls";
import { settingEntry, type SettingsIndexEntry } from "../settings-model";

export const DESKTOP_DRIVING_SETTING = settingEntry({
    id: "agents.desktop_driving",
    labelKey: "settings.agents.desktopDriving.label",
    descriptionKey: "settings.agents.desktopDriving.description",
    section: "agents",
    groupKey: "settings.agents.group.desktopDriving",
    keywords: ["desktop", "desktop driving", "computer use", "native app", "screen", "click", "type", "control", "windows", "desktop:enabled"],
});

export function DesktopDriving(): JSX.Element {
    const settings = () => settingsAtom() ?? ({} as Record<string, unknown>);
    return (
        <SettingRow
            id={DESKTOP_DRIVING_SETTING.id}
            label={DESKTOP_DRIVING_SETTING.label}
            description={DESKTOP_DRIVING_SETTING.description}
            control={<ToggleControl checked={settings()["desktop:enabled"] === true} onChange={(v) => set("desktop:enabled", v)} />}
        />
    );
}
