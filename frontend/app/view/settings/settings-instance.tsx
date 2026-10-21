// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Settings pane's instance, loaded with the first one (settings.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { SETTINGS_SECTION_LABELS, SettingsViewModel, type SettingsSection } from "./settings-model";
import { SettingsView } from "./settings-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    // Read once: the section to open at, not a live binding.
    const asked = ctx.meta()?.["settings:section"];
    const model = new SettingsViewModel(
        typeof asked === "string" && asked in SETTINGS_SECTION_LABELS ? (asked as SettingsSection) : undefined
    );
    return {
        component: () => <SettingsView model={model} />,
        liveTitle: () => ({ text: SETTINGS_SECTION_LABELS[model.activeSection()] }),
    };
}
