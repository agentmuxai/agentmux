// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Toolchain module barrel — wires viewComponent to avoid circular import.

import { ToolchainViewModel } from "./toolchain-model";
import { ToolchainView } from "./toolchain-view";

Object.defineProperty(ToolchainViewModel.prototype, "viewComponent", {
    get() {
        return ToolchainView;
    },
});

export { ToolchainViewModel };
