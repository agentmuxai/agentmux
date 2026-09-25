// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Drone module barrel — wires the view component onto the model
// prototype to avoid a circular import.

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { DroneViewModel } from "./drone-model";
import { DroneView } from "./drone-view";

/** Drone as a native pane tab (Pane Tab contract Phase 2c). */
export const dronePaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "drone",
    // The Workflows feature was renamed to Drone
    // (SPEC_RENAME_WORKFLOWS_TO_DRONE_2026_05_18); persisted blocks still say
    // "workflows".
    aliases: ["workflows"],
    label: "Drone",
    icon: "diagram-project",
    capabilities: { noPadding: true },
    create: (ctx) => {
        const model = new DroneViewModel(ctx);
        return {
            component: () => <DroneView model={model} />,
            liveTitle: () => ({ text: model.viewName() }),
            dispose: () => model.dispose(),
        };
    },
};

export { DroneViewModel };
