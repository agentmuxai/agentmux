// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Swarm pane's instance, loaded with the first one (swarm.tsx).

import { createSignal } from "solid-js";
import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { SwarmViewModel } from "./swarm-model";
import { createSwarmSection, swarmTitleOf } from "./swarm-section";
import { SwarmView } from "./swarm-root";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new SwarmViewModel(ctx.blockId);
    const section = createSwarmSection(ctx.meta);
    // Set by the Kanban section once it is mounted (swarm-kanban-slot.ts); it names the pane.
    const [kanbanNeeds, setKanbanNeeds] = createSignal(0);
    return {
        component: () => <SwarmView model={model} ctx={ctx} kanbanNeeds={kanbanNeeds} onKanbanNeeds={setKanbanNeeds} />,
        // The selected section names the pane: "Swarm · Broadcast", "Swarm · Kanban · 2 need you".
        liveTitle: () => ({ text: swarmTitleOf(section(), kanbanNeeds()) }),
        dispose: () => model.dispose(),
    };
}
