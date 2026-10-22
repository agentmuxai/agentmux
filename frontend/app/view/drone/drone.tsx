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
    // The shared per-pane zoom scales its bar, controls and run panel, not the graph, which has its
    // own zoom (REPORT_PANE_ZOOM_COVERAGE_2026_10_22.md §3.4); 13 px is its text size.
    capabilities: { noPadding: true, paneZoom: { baseFontSize: 13 } },
    members: ["liveTitle"],
    load: () => import("./drone-instance"),
});
