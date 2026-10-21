// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Vault module barrel: its native pane tab manifest. Its code loads with the
// first Vault pane (vault-instance.tsx).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";

/** The Vault as a native pane tab
 *  (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5). */
export const vaultPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "vault",
    label: "Vault",
    icon: "lock",
    defaultHue: 60,
    capabilities: { paneZoom: {} },
    members: ["liveTitle", "headerActions"],
    load: () => import("./vault-instance"),
});
