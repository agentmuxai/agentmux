// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Armory module barrel — wires viewComponent to avoid circular import.

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { ArmoryViewModel } from "./armory-model";
import { ArmoryView } from "./armory-view";

/** Armory as a native pane tab (Pane Tab contract Phase 2c). */
export const armoryPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "armory",
    // The Trust Center was renamed to Armory
    // (docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md);
    // persisted blocks still say "trust".
    aliases: ["trust"],
    label: "Armory",
    icon: "vault",
    // Applies `term:zoom` as CSS zoom.
    capabilities: { paneZoom: {} },
    create: (ctx) => {
        const model = new ArmoryViewModel(ctx);
        return {
            component: () => <ArmoryView model={model} />,
            // The selected section ("Accounts", "Memory", …) names the pane.
            liveTitle: () => ({ text: model.viewName() }),
        };
    },
};

export { ArmoryViewModel };
