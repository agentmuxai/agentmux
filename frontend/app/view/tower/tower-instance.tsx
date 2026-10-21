// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Tower pane's instance, loaded with the first one (tower.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { TowerViewModel } from "./tower-model";
import { TowerView } from "./tower-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new TowerViewModel(ctx);
    return {
        component: () => <TowerView model={model} />,
        liveTitle: () => ({ text: model.viewName() }),
        settingsMenu: () => [
            {
                label: "Share this computer's processes with paired devices",
                type: "checkbox",
                checked: model.sharing(),
                click: () => model.setSharing(!model.sharing()),
            },
        ],
        dispose: () => model.dispose(),
    };
}
