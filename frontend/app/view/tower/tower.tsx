// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tower module barrel: its native pane tab manifest (and the model).

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { TowerViewModel } from "./tower-model";
import { TowerView } from "./tower-view";

/** Tower, the read-only task manager, as a native pane tab. */
export const towerPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "tower",
    label: "Tower",
    icon: "tower-observation",
    defaultHue: 30,
    capabilities: { noPadding: true },
    create: (ctx) => {
        const model = new TowerViewModel(ctx);
        return {
            component: () => <TowerView model={model} />,
            liveTitle: () => ({ text: model.viewName() }),
            dispose: () => model.dispose(),
        };
    },
};

export { TowerViewModel };
