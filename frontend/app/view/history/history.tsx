// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// History module barrel: its native pane tab manifest. Its code loads with the first History pane
// (history-instance.tsx). It was Warden's History section; an old pane, pin or link naming that
// section opens this one (pane-sections.ts).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";

/** History as a native pane tab. One per tab: its widget focuses the open one (action-widgets-config.ts). */
export const historyPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "history",
    label: "History",
    icon: "clock-rotate-left",
    defaultColor: "sky",
    // Applies `term:zoom` as CSS zoom.
    capabilities: { paneZoom: {} },
    members: [],
    load: () => import("./history-instance"),
});
