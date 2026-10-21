// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tower module barrel: its native pane tab manifest. Its code loads with the
// first Tower pane (tower-instance.tsx).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";

/** Tower, the read-only task manager, as a native pane tab. */
export const towerPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "tower",
    label: "Tower",
    icon: "tower-observation",
    defaultHue: 30,
    capabilities: { noPadding: true },
    members: ["liveTitle", "settingsMenu"],
    load: () => import("./tower-instance"),
});
