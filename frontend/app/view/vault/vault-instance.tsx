// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Vault pane's instance, loaded with the first one (vault.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/store/pane-tab-registry";
import { VaultViewModel } from "./vault-model";
import { VaultView } from "./vault-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new VaultViewModel(ctx);
    return {
        component: () => <VaultView model={model} />,
    };
}
