// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Vault module barrel: its native pane tab manifest (and the model).

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { VaultViewModel } from "./vault-model";
import { VaultView } from "./vault-view";

/** The Vault as a native pane tab
 *  (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5). */
export const vaultPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "vault",
    label: "Vault",
    icon: "lock",
    defaultHue: 60,
    capabilities: { paneZoom: {} },
    create: (ctx) => {
        const model = new VaultViewModel(ctx);
        return {
            component: () => <VaultView model={model} />,
            liveTitle: () => ({ text: model.viewName() }),
            headerActions: () => [
                {
                    elemtype: "iconbutton",
                    icon: "file-import",
                    title: "Import into the Vault",
                    click: () => model.openTransfer("import"),
                },
                {
                    elemtype: "iconbutton",
                    icon: "file-export",
                    title: "Export the Vault",
                    click: () => model.openTransfer("export"),
                },
                {
                    elemtype: "iconbutton",
                    icon: "circle-info",
                    title: "What the Vault may ask you",
                    click: () => model.setMeta({ "vault:prompts-read": null }),
                },
            ],
        };
    },
};
