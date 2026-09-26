// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { AgentViewModel } from "./agent-model";
import { AGENT_SPLIT_DROPPED_META, agentPaneTab } from "./agent-pane-tab";
import { AgentBlockContent, buildAgentPaneChromeModel } from "./agent-view";

/**
 * The agent as a native pane tab (Pane Tab contract Phase 2c). In its own
 * module so the model no longer imports its view to hand the host a
 * `viewComponent`: the manifest builds both.
 */
export const agentPaneTabManifest: PaneTabManifest = {
    apiVersion: 1,
    view: "agent",
    // "forge" was folded into the agent pane in v0.33.197.
    aliases: ["forge"],
    label: "Agent",
    icon: "sparkles",
    capabilities: {
        // Keep-alive per the repo owner's decision (SPEC_PANE_TAB_CONTRACT_V1
        // §5): remounting loses the conversation's scroll and composer draft.
        lifecycle: "keepAlive",
        header: "surface",
        paneZoom: {},
        splitDropsMeta: AGENT_SPLIT_DROPPED_META,
        noPadding: true,
    },
    tab: agentPaneTab,
    chrome: buildAgentPaneChromeModel,
    create: (ctx) => {
        const model = new AgentViewModel(ctx);
        return {
            component: () => <AgentBlockContent model={model} />,
            liveTitle: () => ({ text: model.viewName() }),
            rename: (name) => model.setViewName(name),
            headerIcon: () => model.viewIcon(),
            headerActions: () => model.endIconButtons(),
            contextMenu: () => model.getBodyContextMenuItems(),
            // Voice lands beside the composer (AgentFooter), not in the
            // header — the manifest declares no `headerMic` — but
            // Ctrl+Shift+V still targets it.
            voice: () => model.voiceHandle(),
            progressMount: (el) => model.setProgressBarMount(el),
            focus: () => model.giveFocus(),
            dispose: () => model.dispose(),
        };
    },
};
