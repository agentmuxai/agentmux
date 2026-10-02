// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { baseName } from "./files-path";
import { FilesModel, META_PATH } from "./files-model";
import { FilesView } from "./files-view";

/** The pane's title: the folder's name, or "Hangar" before one is shown. */
export function filesTitle(meta: MetaType | undefined): string {
    const path = meta?.[META_PATH];
    if (typeof path !== "string" || path === "") return "Hangar";
    return baseName(path) || path;
}

/**
 * The Files pane, branded Hangar (docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md
 * §13): a native pane tab, kept mounted while another tab of its pane is in
 * front, so scroll and selection survive a switch (§6.2).
 */
export const filesPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "files",
    label: "Hangar",
    icon: "folder-open",
    capabilities: { lifecycle: "keepAlive", noPadding: true },
    create: (ctx) => {
        const model = new FilesModel(ctx);
        return {
            component: () => <FilesView model={model} ctx={ctx} />,
            liveTitle: () => ({ text: filesTitle(ctx.meta()) }),
            // The host calls this when the tab shows in a focused pane.
            focus: () => {
                model.focusList?.();
                return model.focusList != null;
            },
            dispose: () => model.dispose(),
        };
    },
};

export { FilesModel };
