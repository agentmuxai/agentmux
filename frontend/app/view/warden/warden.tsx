// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden module barrel — its native pane tab manifest. Its code loads with
// the first Warden pane (warden-instance.tsx).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";

/** Warden as a native pane tab (Pane Tab contract Phase 2c). */
export const wardenPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "warden",
    label: "Warden",
    icon: "shield-halved",
    defaultHue: 270,
    // Applies `term:zoom` as CSS zoom.
    capabilities: { paneZoom: {} },
    members: ["liveTitle"],
    load: () => import("./warden-instance"),
});
