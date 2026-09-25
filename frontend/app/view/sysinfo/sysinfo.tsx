// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Sysinfo module barrel — its native pane tab manifest (and the model).

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { SysinfoViewModel } from "./sysinfo-model";
import { SysinfoView } from "./sysinfo-view";

/**
 * Sysinfo as a native pane tab (Pane Tab contract Phase 2c). "cpuplot" is its
 * legacy twin: the same view under an older `meta.view` name.
 */
export function sysinfoPaneTab(view: "sysinfo" | "cpuplot"): PaneTabManifest {
    return {
        apiVersion: 1,
        view,
        label: "Sysinfo",
        icon: "chart-line",
        capabilities: { connection: true },
        create: (ctx) => {
            const model = new SysinfoViewModel(ctx, view);
            return {
                component: () => <SysinfoView model={model} blockId={ctx.blockId} />,
                // The plot type ("CPU", "Mem", …) names the pane.
                liveTitle: () => ({ text: model.viewName() }),
                settingsMenu: () => model.getSettingsMenuItems(),
                contextMenu: () => model.getBodyContextMenuItems(),
            };
        },
    };
}

export { SysinfoViewModel };
