// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Memory pane's manifest. Its sections and their code load with the
// first Memory pane (memory-instance.tsx).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";
import { MEMORY_VIEW } from "@/app/view/section-pane/panes";

export const memoryPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: MEMORY_VIEW,
    label: "Memory",
    icon: "brain",
    defaultHue: 150,
    // Applies `term:zoom` as CSS zoom.
    capabilities: { paneZoom: {} },
    members: ["liveTitle"],
    load: () => import("./memory-instance"),
});
