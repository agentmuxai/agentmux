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
    // Zoom is the shared per-pane zoom (keys, Ctrl+scroll, the all-panes gesture); 13 px is its
    // text size. REPORT_PANE_ZOOM_COVERAGE_2026_10_22.md.
    capabilities: { noPadding: true, paneZoom: { baseFontSize: 13 } },
    members: ["liveTitle", "settingsMenu"],
    load: () => import("./tower-instance"),
});
