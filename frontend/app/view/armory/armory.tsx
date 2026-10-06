// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Armory was split into the Connectors and Knowledge panes
// (docs/specs/SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md).
// A block saved as `view: "armory"` (or the older "trust") still loads through
// this manifest, which rewrites the block's meta to its new pane by the section
// it was on, so it loads as that pane from then on. Until the block remounts it
// shows that pane from here: a block's body doesn't follow a `meta.view` change
// while it's mounted. Remove in a later release (the spec's Phase 3).

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { connectorsPaneTab } from "@/app/view/connectors/connectors";
import { knowledgePaneTab } from "@/app/view/knowledge/knowledge";
import { armoryMigrationPatch, CONNECTORS_VIEW } from "@/app/view/section-pane/panes";

export const armoryPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "armory",
    // The Trust Center's view id, from before it was renamed to the Armory.
    aliases: ["trust"],
    label: "Armory",
    icon: "vault",
    // Or Knowledge, by the section it was saved on: create() moves the block.
    legacyOf: "connectors",
    capabilities: { paneZoom: {} },
    create: (ctx) => {
        const patch = armoryMigrationPatch(ctx.meta() as Record<string, unknown>);
        const target = patch.view === CONNECTORS_VIEW ? connectorsPaneTab : knowledgePaneTab;
        // After create() returns: the host is still building this instance.
        queueMicrotask(() => void ctx.setMeta(patch));
        return target.create(ctx);
    },
};
