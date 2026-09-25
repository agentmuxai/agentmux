// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden module barrel — wires viewComponent to avoid circular import.

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { WardenViewModel } from "./warden-model";
import { WardenView } from "./warden-view";

/** Warden as a native pane tab (Pane Tab contract Phase 2c). */
export const wardenPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "warden",
    label: "Warden",
    icon: "shield-halved",
    // Applies `term:zoom` as CSS zoom.
    capabilities: { paneZoom: {} },
    create: (ctx) => {
        const model = new WardenViewModel(ctx);
        return {
            component: () => <WardenView model={model} />,
            // The selected rail section ("Host", "LAN", …) names the pane.
            liveTitle: () => ({ text: model.viewName() }),
        };
    },
};

export { WardenViewModel };
