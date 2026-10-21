// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Warden pane's instance, loaded with the first one (warden.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { vaultHeaderActions } from "@/app/view/vault/vault";
import { WardenViewModel } from "./warden-model";
import { WardenView } from "./warden-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new WardenViewModel(ctx);
    return {
        component: () => <WardenView model={model} />,
        // The selected section ("Host", "LAN", …) names the pane.
        liveTitle: () => ({ text: model.viewName() }),
        // The Vault's own actions, while its section is shown.
        headerActions: () => (model.sectionAtom() === "vault" ? vaultHeaderActions(model.vault) : []),
    };
}
