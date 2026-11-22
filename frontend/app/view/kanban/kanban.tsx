// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Kanban module barrel: its native pane tab manifest. Its code loads with the first Kanban pane
// (kanban-instance.tsx). The board was a Warden section, then a Swarm section; it is a pane of its own
// now, and an old pane, pin or link naming either section opens this one (pane-sections.ts).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";

/** The Kanban as a native pane tab. One per tab: its widget focuses the open one (action-widgets-config.ts). */
export const kanbanPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "kanban",
    // A saved pane, pin or widget that still says "reviews" opens here, on the open-pull-requests filter
    // (kanban-instance.tsx reads the old view name and `reviews:task`).
    aliases: ["reviews"],
    label: "Kanban",
    icon: "table-columns",
    defaultColor: "red",
    // Applies `term:zoom` as CSS zoom; the board fills the pane edge to edge.
    capabilities: { paneZoom: {}, noPadding: true },
    members: ["liveTitle"],
    load: () => import("./kanban-instance"),
});
