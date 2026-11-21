// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Drone pane's instance, loaded with the first one (drone.tsx). Its pane meta `drone:open` (a drone id) is a
// request from elsewhere, the Jobs tab's Open graph: the pane puts that drone's graph on the canvas, then clears it.

import { createEffect } from "solid-js";

import type { PaneTabHostContext, PaneTabInstance } from "@/app/store/pane-tab-registry";
import { DroneViewModel } from "./drone-model";
import { DroneView } from "./drone-view";
import { DRONE_OPEN_META_KEY, droneOpenRequest } from "./open-drone";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new DroneViewModel(ctx);
    return {
        component: () => {
            createEffect(() => {
                const id = droneOpenRequest(ctx.meta());
                if (!id) return;
                // Cleared first, so a second click on the same drone asks again.
                void ctx.setMeta({ [DRONE_OPEN_META_KEY]: null });
                void model.openDrone(id);
            });
            return <DroneView model={model} />;
        },
        liveTitle: () => ({ text: model.viewName() }),
        dispose: () => model.dispose(),
    };
}
