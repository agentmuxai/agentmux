// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

// Forge module barrel — re-exports model and view, wires viewComponent.

import { ForgeViewModel } from "./forge-model";
import { ForgeView } from "./forge-view";

// Wire the view component onto the model prototype to avoid circular imports
// (forge-model.ts cannot import forge-view.tsx without creating a cycle).
Object.defineProperty(ForgeViewModel.prototype, "viewComponent", {
    get() {
        return ForgeView;
    },
});

export { ForgeViewModel };
