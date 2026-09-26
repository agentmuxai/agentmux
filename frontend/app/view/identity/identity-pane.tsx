// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Identity pane barrel: the `view: "identity"` pane's native pane tab
// manifest. Distinct from `identity.tsx` (if any), which barrelled the
// legacy in-agent-pane Identity tab.

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { IdentityPaneView } from "./identity-pane-view";

/** The pane's title: its `frame:title`, else "Identity". */
export function identityTitle(meta: MetaType | undefined): string {
    return (meta?.["frame:title"] as string | undefined) ?? "Identity";
}

/**
 * The identity pane as a native pane tab (Pane Tab contract Phase 2c): one
 * agent's linked accounts, keyed by the block's `meta.agentId`, so it needs
 * no model. `undefined` agent (a generically opened identity block) and the
 * view degrades to a context-free empty state.
 */
export const identityPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "identity",
    label: "Identity",
    icon: "user",
    create: (ctx) => ({
        component: () => <IdentityPaneView agentId={() => ctx.meta()?.["agentId"] as string | undefined} />,
        liveTitle: () => ({ text: identityTitle(ctx.meta()) }),
        headerText: () => "Identity",
    }),
};
