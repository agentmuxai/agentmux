// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Toolchain module barrel: the pane's native pane tab manifest.

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { ToolchainView } from "./toolchain-view";

/** The toolchain pane as a native pane tab (Pane Tab contract Phase 2c). Its
 *  view holds all its own state; there is no model. */
export const toolchainPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "toolchain",
    label: "Toolchain",
    icon: "wrench",
    create: () => ({
        component: () => <ToolchainView />,
        liveTitle: () => ({ text: "Toolchain" }),
    }),
};
