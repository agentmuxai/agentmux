// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Toolchain pane's instance, loaded with the first one (toolchain.tsx).

import type { PaneTabInstance } from "@/app/block/pane-tab-registry";
import { ToolchainView } from "./toolchain-view";

export function create(): PaneTabInstance {
    return {
        component: () => <ToolchainView />,
        liveTitle: () => ({ text: "Toolchain" }),
    };
}
