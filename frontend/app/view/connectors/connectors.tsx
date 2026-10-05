// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Connectors: what agents connect to outside AgentMux. Accounts (sign-ins) and
// MCP servers, the two halves of the Armory it replaced
// (docs/specs/SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md).

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { AccountsManager } from "@/app/view/accounts/accounts-manager";
import { McpManager } from "@/app/view/mcp/mcp-manager";
import { CONNECTORS_SECTION_KEY, CONNECTORS_VIEW, type ConnectorsSection } from "@/app/view/section-pane/panes";
import { SectionPaneModel, SectionPaneView, type SectionPaneSpec } from "@/app/view/section-pane/section-pane";

export const CONNECTORS_SPEC: SectionPaneSpec<ConnectorsSection> = {
    view: CONNECTORS_VIEW,
    sectionKey: CONNECTORS_SECTION_KEY,
    defaultSection: "accounts",
    ariaLabel: "Connectors section",
    sections: [
        {
            id: "accounts",
            label: "Accounts",
            icon: "key",
            tooltip: "Sign-ins to providers and services, bound to agents",
            component: () => <AccountsManager />,
        },
        { id: "mcp", label: "MCP servers", icon: "plug", component: () => <McpManager /> },
    ],
};

export const connectorsPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: CONNECTORS_VIEW,
    label: "Connectors",
    icon: "plug",
    // Applies `term:zoom` as CSS zoom.
    capabilities: { paneZoom: {} },
    create: (ctx) => {
        const model = new SectionPaneModel(ctx, CONNECTORS_SPEC);
        return {
            component: () => <SectionPaneView model={model} />,
            liveTitle: () => ({ text: `Connectors · ${model.viewName()}` }),
        };
    },
};
