// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Editor module barrel — its native pane tab manifest (and the model).

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { EditorViewModel } from "./editor-model";
import { EditorViewComponent } from "./editor-view";
import { editorSplitBlockDef } from "./editor-split";

/** The editor as a native pane tab (Pane Tab contract Phase 2c). */
export const editorPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "editor",
    label: "Editor",
    icon: "file-lines",
    defaultHue: 270,
    // Keep-alive per the repo owner's decision (SPEC_PANE_TAB_CONTRACT_V1 §5):
    // remounting loses the cursor, scroll and undo. Zooms from a 13px base.
    // A split opens an empty editor with this one's settings, not a copy of
    // its documents (SPEC_EDITOR_MEDIA_SPLIT_OPENS_EMPTY_2026_10_10.md).
    capabilities: { lifecycle: "keepAlive", paneZoom: { baseFontSize: 13 }, noPadding: true, splitBlockDef: editorSplitBlockDef },
    create: (ctx) => {
        const model = new EditorViewModel(ctx);
        return {
            component: () => <EditorViewComponent model={model} />,
            liveTitle: () => ({ text: model.viewName() }),
            headerText: () => model.viewText(),
            contextMenu: () => model.getBodyContextMenuItems(),
            focus: () => model.giveFocus(),
            dispose: () => model.dispose(),
        };
    },
};

export { EditorViewModel };
