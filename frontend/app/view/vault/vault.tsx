// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Vault module barrel: its native pane tab manifest. Its code loads with the first Vault pane
// (vault-instance.tsx). The Vault was once a section of Warden; it is a pane of its own, and an old
// pane, pin or link naming Warden's Vault section opens this one (pane-sections.ts).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";
import { VAULT_SECTIONS } from "./vault-kinds";

/** The Vault as a native pane tab. One per tab: its widget focuses the open one (action-widgets-config.ts). */
export const vaultPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "vault",
    label: "Vault",
    icon: "lock",
    defaultColor: "violet",
    // Applies `term:zoom` as CSS zoom (vault-view.tsx).
    capabilities: { paneZoom: {} },
    sections: VAULT_SECTIONS,
    members: ["liveTitle", "headerActions"],
    load: () => import("./vault-instance"),
});
