// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Vault pane's instance, loaded with the first one (vault.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { VaultViewModel } from "./vault-model";
import { VaultView } from "./vault-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
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
}
