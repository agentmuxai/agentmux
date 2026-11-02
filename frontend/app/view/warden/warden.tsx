// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden module barrel: its native pane tab manifest. Its code loads with the first Warden pane
// (warden-instance.tsx). Only Jobs is left of it: the Vault, History and Audit are panes of their own
// (pane-sections.ts maps an old link to each).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";
import { WARDEN_SECTIONS } from "./warden-sections";

/** Warden as a native pane tab. */
export const wardenPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "warden",
    label: "Jobs",
    icon: "calendar-check",
    defaultColor: "violet",
    // A click on Jobs brings back the one already open in the tab.
    singleton: true,
    // Applies `term:zoom` as CSS zoom.
    capabilities: { paneZoom: {} },
    sections: WARDEN_SECTIONS,
    members: ["liveTitle"],
    load: () => import("./warden-instance"),
});
