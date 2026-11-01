// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// Editor module barrel — its native pane tab manifest. Its code (CodeMirror
// included) loads with the first Editor pane (editor-instance.tsx).

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";
import { editorSplitBlockDef } from "./editor-split";

/** The editor as a native pane tab (Pane Tab contract Phase 2c). */
export const editorPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "editor",
    // The Media pane is folded into the Editor: a saved Media pane opens as
    // an Editor, its files as tabs following their folder (the model's
    // migrateMediaMeta). SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.12.
    aliases: ["media"],
    label: "Editor",
    icon: "file-lines",
    defaultColor: "violet",
    // Keep-alive per the repo owner's decision (SPEC_PANE_TAB_CONTRACT_V1 §5):
    // remounting loses the cursor, scroll and undo. Zooms from a 13px base.
    // A split opens an empty editor with this one's settings, not a copy of
    // its documents (SPEC_EDITOR_MEDIA_SPLIT_OPENS_EMPTY_2026_10_10.md).
    capabilities: { lifecycle: "keepAlive", paneZoom: { baseFontSize: 13 }, noPadding: true, splitBlockDef: editorSplitBlockDef },
    members: ["liveTitle", "headerText", "contextMenu", "focus"],
    load: () => import("./editor-instance"),
});
