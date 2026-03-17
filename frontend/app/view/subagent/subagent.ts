// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

// Subagent module barrel — re-exports model and view, wires viewComponent.

import { SubagentViewModel } from "./subagent-model";
import { SubagentView } from "./subagent-view";

// Wire the view component onto the model prototype to avoid circular imports
// (subagent-model.ts cannot import subagent-view.tsx without creating a cycle).
Object.defineProperty(SubagentViewModel.prototype, "viewComponent", {
    get() {
        return SubagentView;
    },
});

export { SubagentViewModel };
