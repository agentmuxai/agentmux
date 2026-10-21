// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Settings pane's instance, loaded with the first one (settings.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { SETTINGS_SECTION_LABELS, SettingsViewModel } from "./settings-model";
import { SettingsView } from "./settings-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new SettingsViewModel(ctx);
    return {
        component: () => <SettingsView model={model} />,
        liveTitle: () => ({ text: SETTINGS_SECTION_LABELS[model.activeSection()] }),
    };
}
