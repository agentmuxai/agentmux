// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The layout a pane or pane-tab drag began in, as the dragged pane's picture needs it
 * (app/drag/pane-live-follow.ts): its body on screen, where the picture stays out of sight because the layout
 * itself shows the pane, and whether it shows the pane now. It does not once the drag has gone on to another
 * window tab, whose layout shows nothing of the pane, nor while it shows the tear-off (tearOffPreview.ts).
 */

import type { HomeLayout } from "@/app/drag/pane-live-follow";
import { atoms } from "@/store/global";
import type { LayoutModel } from "./layoutModel";

/** `model`'s layout as a drag's home, for a drag that is on while `active` holds. */
export function homeLayoutOf(model: Pick<LayoutModel, "displayContainerRef" | "tabAtom" | "tearOffLayout">, active: () => boolean): HomeLayout {
    return {
        body: () => {
            const rect = model.displayContainerRef?.current?.getBoundingClientRect();
            if (!rect || !(rect.width > 0) || !(rect.height > 0)) return undefined;
            return { left: rect.left, top: rect.top, width: rect.width, height: rect.height };
        },
        // Not while it shows the tear-off (the others closed over the pane's place): the pane is in no place then.
        holdsPane: () => atoms.activeTabId() === model.tabAtom()?.oid && !model.tearOffLayout(),
        active,
    };
}
