// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Toolchain pane's instance, loaded with the first one (toolchain.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { readZoom } from "@/app/store/zoom-factor";
import { ToolchainView } from "./toolchain-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    return {
        component: () => <ToolchainView zoom={() => readZoom(ctx.meta())} />,
        liveTitle: () => ({ text: "Toolchain" }),
    };
}
