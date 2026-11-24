// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Audit module barrel: its native pane tab manifest. Its code loads with the first Audit pane
// (audit-instance.tsx). It was Warden's Audit section; an old pane, pin or link naming that section
// opens this one (pane-sections.ts).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";
import { AUDIT_SECTIONS } from "./audit-sections";

/** The Audit as a native pane tab. One per tab: its widget focuses the open one (action-widgets-config.ts). */
export const auditPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "audit",
    label: "Audit",
    icon: "list-check",
    defaultColor: "amber",
    // Applies `term:zoom` as CSS zoom.
    capabilities: { paneZoom: {} },
    // Events, Conversations, Typed and Storage. The pane's title is its label, whichever section it shows.
    sections: AUDIT_SECTIONS,
    members: [],
    load: () => import("./audit-instance"),
});
