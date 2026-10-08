// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Connectors: what agents connect to outside AgentMux. Accounts (sign-ins) and
// MCP servers, the two halves of the Armory it replaced
// (docs/specs/SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md),
// and Remotes, once a pane of its own
// (docs/specs/SPEC_REMOTES_INTO_CONNECTORS_2026_10_08.md).

import { createEffect, createSignal, onCleanup, Show, type JSX } from "solid-js";

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { AccountsManager } from "@/app/view/accounts/accounts-manager";
import { McpManager } from "@/app/view/mcp/mcp-manager";
import { RemotesViewModel } from "@/app/view/remotes/remotes-model";
import { RemotesView } from "@/app/view/remotes/remotes-view";
import { CONNECTORS_SECTION_KEY, CONNECTORS_VIEW, type ConnectorsSection } from "@/app/view/section-pane/panes";
import { SectionPaneModel, SectionPaneView, type SectionPaneSpec } from "@/app/view/section-pane/section-pane";

/** The Remotes list, on this pane's block: New terminal splits beside it, and
 *  another pane's link reaches it through `remotes:expand` (open-remotes.ts). */
function RemotesList(props: { pane: SectionPaneModel<ConnectorsSection> }): JSX.Element {
    const pane = props.pane;
    const model = new RemotesViewModel({
        blockId: pane.blockId,
        meta: pane.meta,
        setMeta: async (p) => pane.setMeta(p),
    });
    onCleanup(() => model.dispose());
    return <RemotesView model={model} />;
}

/** Built the first time the section is shown, then kept like the others: a
 *  Connectors pane left on Accounts doesn't load and follow the remotes list. */
function RemotesSection(props: { pane: SectionPaneModel<ConnectorsSection> }): JSX.Element {
    const [shown, setShown] = createSignal(false);
    createEffect(() => {
        if (props.pane.sectionAtom() === "remotes") setShown(true);
    });
    return (
        <Show when={shown()}>
            <RemotesList pane={props.pane} />
        </Show>
    );
}

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
        {
            id: "remotes",
            label: "Remotes",
            icon: "server",
            tooltip: "Remote machines (SSH, WSL)",
            component: (pane) => <RemotesSection pane={pane} />,
        },
    ],
};

export const connectorsPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: CONNECTORS_VIEW,
    label: "Connectors",
    icon: "plug",
    defaultHue: 90,
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
