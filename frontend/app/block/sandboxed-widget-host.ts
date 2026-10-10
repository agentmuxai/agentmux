// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane host for a sandboxed widget
 * (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6). Phase W1
 * lists, approves and serves sandboxed packages; the iframe runtime and its
 * bridge arrive in W2, so until then a sandboxed pane says so plainly rather
 * than failing.
 */

import type { WidgetPackageInfo, WidgetPaneInfo } from "@/app/store/rpc-api/widgets";
import type { PaneTabManifest } from "./pane-tab-registry";

export function makeSandboxedPaneManifest(pkg: WidgetPackageInfo, pane: WidgetPaneInfo): PaneTabManifest {
    return {
        apiVersion: 1,
        view: pane.view,
        label: pane.label,
        icon: pane.icon,
        defaultHue: pkg.default_hue ?? undefined,
        create: () => ({
            component: () => {
                const el = document.createElement("div");
                el.className = "sandboxed-widget-pending";
                el.style.padding = "16px";
                el.textContent = `${pkg.name} is a sandboxed widget. This build of AgentMux can install and approve it, but can't run sandboxed widgets yet.`;
                return el;
            },
            dispose: () => {},
        }),
    };
}
