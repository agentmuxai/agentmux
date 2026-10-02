// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { getObjectValue, makeORef } from "@/app/store/mos";
import { baseName } from "./files-path";
import { FilesModel, META_PATH } from "./files-model";
import { FilesPane, FilesPaneModel } from "./files-pane";

/** The pane's title: the folder's name, or "Hangar" before one is shown. */
export function filesTitle(meta: MetaType | undefined): string {
    const path = meta?.[META_PATH];
    if (typeof path !== "string" || path === "") return "Hangar";
    return baseName(path) || path;
}

/**
 * The Files pane, branded Hangar (docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md
 * §13): a native pane tab, kept mounted while another pane tab of its pane is
 * in front, so scroll and selection survive a switch (§6.2). Its folders are
 * document tabs inside it.
 */
export const filesPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "files",
    label: "Hangar",
    icon: "folder-open",
    capabilities: { lifecycle: "keepAlive", noPadding: true },
    // A Hangar tab added with the pane's "+" starts in the folder the tab in
    // front shows, as a terminal's new tab starts in its directory.
    chrome: (_anchor, nodeModel) => ({
        newTabMeta: (view) => {
            if (view !== "files") return undefined;
            const active = nodeModel.activeBlockId?.() ?? nodeModel.blockId;
            const path = getObjectValue<Block>(makeORef("block", active))?.meta?.[META_PATH];
            return typeof path === "string" && path !== "" ? { [META_PATH]: path } : undefined;
        },
    }),
    // Its folders are document tabs inside the pane
    // (SPEC_DOCUMENT_TABS_2026_10_02.md §6.2).
    create: (ctx) => {
        const pane = new FilesPaneModel(ctx);
        return {
            component: () => <FilesPane pane={pane} ctx={ctx} />,
            liveTitle: () => ({ text: pane.title() }),
            // The host calls this when the tab shows in a focused pane.
            focus: () => {
                const model = pane.activeModel();
                model?.focusList?.();
                return model?.focusList != null;
            },
            dispose: () => pane.dispose(),
        };
    },
};

export { FilesModel };
