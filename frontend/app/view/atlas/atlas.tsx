// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Atlas module barrel: its native pane tab manifest. Its code loads with the first Atlas pane (atlas-instance.tsx).
import { lazyPaneTab } from "@/app/block/lazy-pane-tab";

/** Atlas as a native pane tab: the Swarm's agents as a map (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md). */
export const atlasPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "atlas",
    label: "Atlas",
    title: () => "Atlas",
    icon: "map",
    defaultColor: "red",
    // Applies `term:zoom` as CSS zoom; the map fills the pane edge to edge.
    capabilities: { paneZoom: {}, noPadding: true },
    members: ["liveTitle"],
    load: () => import("./atlas-instance"),
});
