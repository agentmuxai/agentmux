// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory module barrel — wires the view component onto the model
// prototype to avoid a circular import between memory-model.ts and
// memory-view.tsx.

import { MemoryViewModel } from "./memory-model";
import { MemoryView } from "./memory-view";

Object.defineProperty(MemoryViewModel.prototype, "viewComponent", {
    get() {
        return MemoryView;
    },
});

export { MemoryViewModel };
