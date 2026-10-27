// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Swarm pane's instance, loaded with the first one (swarm.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { SwarmViewModel } from "./swarm-model";
import { createSwarmSection, swarmTitleOf } from "./swarm-section";
import { SwarmView } from "./swarm-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new SwarmViewModel(ctx.blockId);
    const section = createSwarmSection(ctx.meta);
    return {
        component: () => <SwarmView model={model} ctx={ctx} />,
        // The selected section names the pane: "Swarm · Broadcast".
        liveTitle: () => ({ text: swarmTitleOf(section()) }),
        dispose: () => model.dispose(),
    };
}
