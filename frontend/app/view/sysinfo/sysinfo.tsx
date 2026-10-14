// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Sysinfo module barrel — its native pane tab manifest (and the model).

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { remoteOf, remoteTabIcon, remoteTabName } from "@/app/block/pane-tab-remote";
import { SysinfoViewModel } from "./sysinfo-model";
import { DefaultPlotType } from "./sysinfo-types";
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
        ...(view === "sysinfo" ? { defaultHue: 300 } : { legacyOf: "sysinfo" }),
        capabilities: { connection: true, noPadding: true },
        // On a remote machine the pill says which ("CPU · devbox", the
        // remote's icon); on this one it keeps the plot type from liveTitle.
        tab: {
            label: ({ meta }) => {
                const remote = remoteOf(meta);
                if (!remote) return undefined;
                const plot = (meta?.["sysinfo:type"] as string | undefined) || DefaultPlotType;
                return `${plot} · ${remoteTabName(remote)}`;
            },
            icon: ({ meta }) => {
                const remote = remoteOf(meta);
                return remote ? remoteTabIcon(remote) : undefined;
            },
        },
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
