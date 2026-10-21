// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings module barrel: the pane's native pane tab manifest. Its code loads
// with the first Settings pane (settings-instance.tsx).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";
import { SETTINGS_SECTIONS } from "./settings-model";

/** The settings pane as a native pane tab (Pane Tab contract Phase 2c),
 *  titled with its open section. */
export const settingsPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "settings",
    label: "Settings",
    icon: "cog",
    defaultHue: 0,
    sections: SETTINGS_SECTIONS,
    members: ["liveTitle"],
    load: () => import("./settings-instance"),
});
