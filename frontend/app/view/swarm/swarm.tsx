// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Swarm module barrel: its native pane tab manifest. Its code loads with the
// first Swarm pane (swarm-instance.tsx).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";
import { SWARM_SECTIONS } from "./swarm-section";

/** Swarm as a native pane tab (Pane Tab contract Phase 2c). */
export const swarmPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "swarm",
    label: "Swarm",
    icon: "diagram-project",
    defaultHue: 0,
    // Applies `term:zoom` as CSS zoom; fills the pane edge to edge.
    capabilities: { paneZoom: {}, noPadding: true },
    sections: SWARM_SECTIONS,
    members: ["liveTitle", "search"],
    load: () => import("./swarm-instance"),
});
