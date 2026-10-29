// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { VaultViewModel } from "./vault-model";

/** Import, export and the note on what may ask the person, in the Vault pane's header. */
export function vaultHeaderActions(model: VaultViewModel): IconButtonDecl[] {
    return [
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
    ];
}
