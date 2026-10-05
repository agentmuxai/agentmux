// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Armory was split into the Connectors and Knowledge panes
// (docs/specs/SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md).
// A block saved as `view: "armory"` (or the older "trust") still loads through
// this manifest, which rewrites the block's meta to its new pane by the section
// it was on; the block then remounts as that pane. Remove in a later release
// (the spec's Phase 3).

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { armoryMigrationPatch } from "@/app/view/section-pane/panes";

export const armoryPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "armory",
    // The Trust Center's view id, from before it was renamed to the Armory.
    aliases: ["trust"],
    label: "Armory",
    icon: "vault",
    create: (ctx) => {
        // After create() returns: the host is still building this instance.
        queueMicrotask(() => void ctx.setMeta(armoryMigrationPatch(ctx.meta() as Record<string, unknown>)));
        return {
            component: () => <div class="armory-migrating" />,
            liveTitle: () => ({ text: "Armory", placeholder: true }),
        };
    },
};
