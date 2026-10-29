// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Swarm pane's instance, loaded with the first one (swarm.tsx).

import { createEffect } from "solid-js";
import { movedMeta } from "@/app/block/pane-sections";
import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { SwarmViewModel } from "./swarm-model";
import { createSwarmSection, swarmTitleOf } from "./swarm-section";
import { SwarmView } from "./swarm-root";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new SwarmViewModel(ctx.blockId);
    const section = createSwarmSection(ctx.meta);
    // A saved Swarm that was on its Kanban section becomes the Kanban pane (the board left the Swarm,
    // #9670). Its `kanban:*` meta is the same block's, so lanes and collapsed columns carry over.
    createEffect(() => {
        const meta = ctx.meta();
        const moved = meta ? movedMeta(meta) : meta;
        if (meta && moved !== meta) void ctx.setMeta({ view: moved?.view, "swarm:section": null });
    });
    return {
        component: () => <SwarmView model={model} ctx={ctx} />,
        // The selected section names the pane: "Swarm · Broadcast".
        liveTitle: () => ({ text: swarmTitleOf(section()) }),
        dispose: () => model.dispose(),
    };
}
