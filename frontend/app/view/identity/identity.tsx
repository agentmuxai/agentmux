// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Identity module barrel — re-exports model and view, wires viewComponent.

import { IdentityViewModel } from "./identity-model";
import { IdentityView } from "./identity-view";

// Wire the view component onto the model prototype to avoid circular imports
// (identity-model.ts cannot import identity-view.tsx without creating a cycle).
Object.defineProperty(IdentityViewModel.prototype, "viewComponent", {
    get() {
        return IdentityView;
    },
});

export { IdentityViewModel };
