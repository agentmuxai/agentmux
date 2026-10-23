// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Toolchain module barrel: its native pane tab manifest. Its code loads with
// the first Toolchain pane (toolchain-instance.tsx).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";

/** The toolchain pane as a native pane tab (Pane Tab contract Phase 2c). Its
 *  view holds all its own state; there is no model. */
export const toolchainPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "toolchain",
    label: "Toolchain",
    icon: "wrench",
    defaultHue: 210,
    // The shared per-pane zoom; 12 px is its text size. REPORT_PANE_ZOOM_COVERAGE_2026_10_22.md.
    capabilities: { paneZoom: { baseFontSize: 12 } },
    members: ["liveTitle"],
    load: () => import("./toolchain-instance"),
});
