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
import type { SwarmFilterFocus } from "./swarm-filter-bar";
import { createSignalAtom } from "@/util/util";

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
    // Find in pane (Ctrl/Cmd+F) is the filter: opening the find "bar" puts the focus in its box,
    // and the bar is always there, so it closes again at once (swarm-filter-bar.tsx).
    let filterBox: SwarmFilterFocus | undefined;
    const find = createSignalAtom(false);
    createEffect(() => {
        if (find()) {
            filterBox?.focus();
            find._set(false);
        }
    });
    const searchAtoms: SearchAtoms = {
        searchValue: createSignalAtom(""),
        resultsIndex: createSignalAtom(0),
        resultsCount: createSignalAtom(0),
        isOpen: find,
    };
    return {
        component: () => <SwarmView model={model} ctx={ctx} filterFocusRef={(focus) => (filterBox = focus)} />,
        search: () => searchAtoms,
        // The selected section names the pane: "Swarm · Broadcast".
        liveTitle: () => ({ text: swarmTitleOf(section()) }),
        dispose: () => model.dispose(),
    };
}
