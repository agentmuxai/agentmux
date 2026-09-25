// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { SwarmViewModel } from "./swarm-model";
import { SwarmView } from "./swarm-view";

/** Swarm as a native pane tab (Pane Tab contract Phase 2c). */
export const swarmPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "swarm",
    label: "Swarm",
    icon: "diagram-project",
    // Applies `term:zoom` as CSS zoom; fills the pane edge to edge.
    capabilities: { paneZoom: {}, noPadding: true },
    create: (ctx) => {
        const model = new SwarmViewModel(ctx.blockId);
        return {
            component: () => <SwarmView model={model} ctx={ctx} />,
            dispose: () => model.dispose(),
        };
    },
};

export { SwarmViewModel };
