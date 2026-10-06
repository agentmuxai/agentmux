// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { RemotesViewModel } from "./remotes-model";
import { RemotesView } from "./remotes-view";

/** Remotes as a pane tab: an ordinary widget (SPEC_REMOTES_PANE_2026_10_05.md §4.1). */
export const remotesPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "remotes",
    label: "Remotes",
    icon: "server",
    defaultHue: 210,
    // Applies `term:zoom` as CSS zoom. The default lifecycle (remount): the
    // list is srv's, so nothing is lost when an inactive tab unmounts.
    capabilities: { paneZoom: {} },
    create: (ctx) => {
        const model = new RemotesViewModel(ctx);
        return {
            component: () => <RemotesView model={model} />,
            dispose: () => model.dispose(),
        };
    },
};

export { RemotesViewModel };
