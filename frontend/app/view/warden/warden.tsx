// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden module barrel — its native pane tab manifest (and the model).

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { vaultHeaderActions } from "@/app/view/vault/vault";
import { WARDEN_SECTIONS, WardenViewModel } from "./warden-model";
import { WardenView } from "./warden-view";

/** Warden as a native pane tab (Pane Tab contract Phase 2c). */
export const wardenPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "warden",
    label: "Warden",
    icon: "shield-halved",
    defaultHue: 270,
    // Applies `term:zoom` as CSS zoom.
    // Blocks saved when the Vault was a pane of its own open on its section.
    aliases: ["vault"],
    capabilities: { paneZoom: {} },
    sections: WARDEN_SECTIONS,
    create: (ctx) => {
        const model = new WardenViewModel(ctx);
        return {
            component: () => <WardenView model={model} />,
            // The selected section ("Host", "LAN", …) names the pane.
            liveTitle: () => ({ text: model.viewName() }),
            // The Vault's own actions, while its section is shown.
            headerActions: () => (model.sectionAtom() === "vault" ? vaultHeaderActions(model.vault) : []),
        };
    },
};
