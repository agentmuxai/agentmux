// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory module barrel: the `view: "memory"` pane's native pane tab manifest,
// and the bundle model the context-free `BundleManager` edits bundles with.

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { BundleViewModel } from "./bundle-model";
import { BundleView } from "./bundle-view";

/** The pane's title: its `frame:title`, else "Bundles". */
export function memoryTitle(meta: MetaType | undefined): string {
    return (meta?.["frame:title"] as string | undefined) ?? "Bundles";
}

/**
 * The memory pane as a native pane tab (Pane Tab contract Phase 2c). A
 * read-only summary of one agent's bundle, keyed by the block's
 * `meta.agentId`, so it needs no model: it used to be backed by a full
 * `BundleViewModel`, which listed every bundle and subscribed to changes on
 * construction for nothing this pane shows.
 */
export const memoryPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "memory",
    label: "Memory",
    icon: "layer-group",
    create: (ctx) => ({
        component: () => <BundleView agentId={() => ctx.meta()?.["agentId"] as string | undefined} />,
        liveTitle: () => ({ text: memoryTitle(ctx.meta()) }),
        headerText: () => "Bundles",
    }),
};

export { BundleViewModel };
