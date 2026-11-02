// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { lazyPaneTab } from "@/app/block/lazy-pane-tab";
import { getObjectValue, makeORef } from "@/app/store/mos";
import { baseName, META_PATH } from "./files-path";

/** The pane's title: the folder's name, or "Files" before one is shown. */
export function filesTitle(meta: MetaType | undefined): string {
    const path = meta?.[META_PATH];
    if (typeof path !== "string" || path === "") return "Files";
    return baseName(path) || path;
}

/**
 * The Files pane (docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md
 * §13): a native pane tab, kept mounted while another tab of its pane is in
 * front, so scroll and selection survive a switch (§6.2). Its code loads with
 * the first Files pane (files-instance.tsx).
 */
export const filesPaneTab = lazyPaneTab({
    apiVersion: 1,
    view: "files",
    label: "Files",
    title: filesTitle,
    icon: "folder-open",
    defaultColor: "cyan",
    // Zoom is the shared per-pane zoom (keys, Ctrl+scroll, the all-panes
    // gesture); 12 px is the Files pane's font size. SPEC_FILES_ZOOM_2026_10_08.md.
    capabilities: { lifecycle: "keepAlive", noPadding: true, paneZoom: { baseFontSize: 12 } },
    // A Files tab added with the pane's "+" starts in the folder the tab in
    // front shows, as a terminal's new tab starts in its directory.
    chrome: (_anchor, nodeModel) => ({
        newTabMeta: (view) => {
            if (view !== "files") return undefined;
            const active = nodeModel.activeBlockId?.() ?? nodeModel.blockId;
            const path = getObjectValue<Block>(makeORef("block", active))?.meta?.[META_PATH];
            return typeof path === "string" && path !== "" ? { [META_PATH]: path } : undefined;
        },
    }),
    members: ["liveTitle", "focus"],
    load: () => import("./files-instance"),
});
