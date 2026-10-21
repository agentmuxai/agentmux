// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Drone pane's instance, loaded with the first one (drone.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { DroneViewModel } from "./drone-model";
import { DroneView } from "./drone-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new DroneViewModel(ctx);
    return {
        component: () => <DroneView model={model} />,
        liveTitle: () => ({ text: model.viewName() }),
        dispose: () => model.dispose(),
    };
}
