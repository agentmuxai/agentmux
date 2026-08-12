// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden module barrel — wires viewComponent to avoid circular import.

import { WardenViewModel } from "./warden-model";
import { WardenView } from "./warden-view";

Object.defineProperty(WardenViewModel.prototype, "viewComponent", {
    get() {
        return WardenView;
    },
});

export { WardenViewModel };
