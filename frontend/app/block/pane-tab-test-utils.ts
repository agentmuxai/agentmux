// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Test-only: a minimal native pane tab manifest, for tests that need a view
 * type registered (its label, icon, capabilities, tab descriptor or chrome)
 * but never build a live instance of it.
 */

import type { PaneTabDescriptor } from "@/app/element/pane-tab-model";
import type { PaneTabCapabilities, PaneTabManifest } from "./pane-tab-registry";

export function stubPaneTab(
    view: string,
    opts: {
        label?: string;
        icon?: string;
        aliases?: string[];
        lifecycle?: PaneTabCapabilities["lifecycle"];
        capabilities?: Omit<PaneTabCapabilities, "lifecycle">;
        tab?: PaneTabDescriptor;
        chrome?: PaneTabManifest["chrome"];
    } = {}
): PaneTabManifest {
    return {
        apiVersion: 1,
        view,
        aliases: opts.aliases,
        label: opts.label ?? view,
        icon: opts.icon ?? "square",
        capabilities:
            opts.lifecycle || opts.capabilities
                ? { ...opts.capabilities, ...(opts.lifecycle ? { lifecycle: opts.lifecycle } : {}) }
                : undefined,
        tab: opts.tab,
        chrome: opts.chrome,
        create: () => ({ component: () => null }),
    };
}
