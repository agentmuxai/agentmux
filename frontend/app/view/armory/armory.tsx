// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Armory module barrel — wires viewComponent to avoid circular import.

import { ArmoryViewModel } from "./armory-model";
import { ArmoryView } from "./armory-view";

Object.defineProperty(ArmoryViewModel.prototype, "viewComponent", {
    get() {
        return ArmoryView;
    },
});

export { ArmoryViewModel };
