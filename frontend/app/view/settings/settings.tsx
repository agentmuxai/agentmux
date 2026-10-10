// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings module barrel: the pane's native pane tab manifest.

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { SETTINGS_SECTION_LABELS, SettingsViewModel, type SettingsSection } from "./settings-model";
import { SettingsView } from "./settings-view";

/** The settings pane as a native pane tab (Pane Tab contract Phase 2c),
 *  titled with its open section. */
export const settingsPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "settings",
    label: "Settings",
    icon: "cog",
    defaultHue: 0,
    create: (ctx) => {
        // Read once: the section to open at, not a live binding.
        const asked = ctx.meta()?.["settings:section"];
        const model = new SettingsViewModel(
            typeof asked === "string" && asked in SETTINGS_SECTION_LABELS ? (asked as SettingsSection) : undefined
        );
        return {
            component: () => <SettingsView model={model} />,
            liveTitle: () => ({ text: SETTINGS_SECTION_LABELS[model.activeSection()] }),
        };
    },
};

export { SettingsViewModel };
