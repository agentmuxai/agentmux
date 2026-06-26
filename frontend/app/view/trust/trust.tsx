// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Trust Center module barrel — wires viewComponent to avoid circular import.

import { TrustViewModel } from "./trust-model";
import { TrustView } from "./trust-view";

Object.defineProperty(TrustViewModel.prototype, "viewComponent", {
    get() {
        return TrustView;
    },
});

export { TrustViewModel };
