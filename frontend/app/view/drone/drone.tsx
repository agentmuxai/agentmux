// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Drone module barrel: its native pane tab manifest. Its code loads with the
// first Drone pane (drone-instance.tsx).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";

/** Drone as a native pane tab (Pane Tab contract Phase 2c). */
export const dronePaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "drone",
    // The Workflows feature was renamed to Drone
    // (SPEC_RENAME_WORKFLOWS_TO_DRONE_2026_05_18); persisted blocks still say
    // "workflows".
    aliases: ["workflows"],
    label: "Drone",
    icon: "diagram-project",
    defaultHue: 240,
    capabilities: { noPadding: true },
    members: ["liveTitle"],
    load: () => import("./drone-instance"),
});
