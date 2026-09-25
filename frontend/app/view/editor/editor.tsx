// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Editor module barrel — wires viewComponent onto EditorViewModel.

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { EditorViewModel } from "./editor-model";
import { EditorViewComponent } from "./editor-view";

/** The editor as a native pane tab (Pane Tab contract Phase 2c). */
export const editorPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "editor",
    label: "Editor",
    icon: "file-lines",
    // Keep-alive per the repo owner's decision (SPEC_PANE_TAB_CONTRACT_V1 §5):
    // remounting loses the cursor, scroll and undo. Zooms from a 13px base.
    capabilities: { lifecycle: "keepAlive", paneZoom: { baseFontSize: 13 }, noPadding: true },
    create: (ctx) => {
        const model = new EditorViewModel(ctx);
        return {
            component: () => <EditorViewComponent model={model} />,
            liveTitle: () => ({ text: model.viewName() }),
            headerText: () => model.viewText(),
            headerIcon: () => model.viewIcon(),
            contextMenu: () => model.getBodyContextMenuItems(),
            focus: () => model.giveFocus(),
            dispose: () => model.dispose(),
        };
    },
};

export { EditorViewModel };
