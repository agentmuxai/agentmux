// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Connectors pane's manifest. Its sections and their code load with the
// first Connectors pane (connectors-instance.tsx).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";
import { CONNECTORS_SECTIONS, CONNECTORS_VIEW } from "@/app/view/section-pane/panes";

export const connectorsPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: CONNECTORS_VIEW,
    label: "Connectors",
    icon: "plug",
    defaultColor: "green",
    // Applies `term:zoom` as CSS zoom.
    capabilities: { paneZoom: {} },
    sections: CONNECTORS_SECTIONS,
    members: ["liveTitle"],
    load: () => import("./connectors-instance"),
});
