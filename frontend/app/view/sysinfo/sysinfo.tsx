// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Sysinfo module barrel — re-exports model and view, wires viewComponent.

import { SysinfoViewModel } from "./sysinfo-model";
import { SysinfoView } from "./sysinfo-view";

// Wire the view component onto the model prototype to avoid circular imports
// (sysinfo-model.ts cannot import sysinfo-view.tsx without creating a cycle).
Object.defineProperty(SysinfoViewModel.prototype, "viewComponent", {
    get() {
        return SysinfoView;
    },
});

export { SysinfoViewModel };
