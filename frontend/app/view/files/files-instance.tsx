// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Files pane's instance, loaded with the first one (files.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { filesTitle } from "./files";
import { FilesModel } from "./files-model";
import { FilesView } from "./files-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new FilesModel(ctx);
    return {
        component: () => <FilesView model={model} ctx={ctx} />,
        liveTitle: () => ({ text: filesTitle(ctx.meta()) }),
        // The host calls this when the tab shows in a focused pane.
        focus: () => {
            model.focusList?.();
            return model.focusList != null;
        },
        dispose: () => model.dispose(),
    };
}
