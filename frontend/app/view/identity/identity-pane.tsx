// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Identity pane barrel — wires the view component onto the standalone-
// pane ViewModel prototype. Distinct from `identity.tsx` (if any) which
// barrelled the legacy in-agent-pane Identity tab.

import { IdentityPaneViewModel } from "./identity-pane-model";
import { IdentityPaneView } from "./identity-pane-view";

Object.defineProperty(IdentityPaneViewModel.prototype, "viewComponent", {
    get() {
        return IdentityPaneView;
    },
});

export { IdentityPaneViewModel };
