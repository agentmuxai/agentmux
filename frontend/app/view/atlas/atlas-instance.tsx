// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An Atlas pane's instance, loaded with the first one (atlas.tsx). It reads the Swarm's own model, so the map and
// the list agree and no second data path exists.

import type { PaneTabHostContext, PaneTabInstance } from "@/app/store/pane-tab-registry";
import { SwarmViewModel } from "../swarm/swarm-model";
import { AtlasView } from "./atlas-view";
import { useAtlasData } from "./atlas-model";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new SwarmViewModel(ctx.blockId);
    const data = useAtlasData(model, ctx);
    return {
        component: () => <AtlasView data={data} ctx={ctx} />,
        liveTitle: () => ({ text: "Atlas" }),
        dispose: () => model.dispose(),
    };
}
