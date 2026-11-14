// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The layout a pane or pane-tab drag began in, as the dragged pane's picture needs it
 * (app/drag/pane-live-follow.ts): its body on screen, where the picture stays out of sight because the layout
 * itself shows the pane, and whether it shows the pane now. It does not once the drag has gone on to another
 * window tab, whose layout shows nothing of the pane, nor while it shows the tear-off (tearOffPreview.ts), nor
 * while the pane's own place still offers the tear-off: that place is the red one, with nothing of the pane in
 * it, and the hand had only the pointer there (the operator, 2026-11-13, of the same for a dragged tab: "the
 * background is correct, but we lost the follow window, we only see a special pointer"; then "works for pane
 * tabs. Now fix it for panes").
 */

import type { HomeLayout } from "@/app/drag/pane-live-follow";
import { atoms } from "@/store/global";
import type { LayoutModel } from "./layoutModel";

/** `model`'s layout as a drag's home, for a drag that is on while `active` holds. */
export function homeLayoutOf(
    model: Pick<LayoutModel, "displayContainerRef" | "tabAtom" | "tearOffLayout" | "tearOffOffered">,
    active: () => boolean
): HomeLayout {
    return {
        body: () => {
            const rect = model.displayContainerRef?.current?.getBoundingClientRect();
            if (!rect || !(rect.width > 0) || !(rect.height > 0)) return undefined;
            return { left: rect.left, top: rect.top, width: rect.width, height: rect.height };
        },
        // Not while it shows the tear-off (the others closed over the pane's place), nor while the pane's own
        // place offers it (the red place): the pane is in no place then.
        holdsPane: () => atoms.activeTabId() === model.tabAtom()?.oid && !model.tearOffLayout() && !model.tearOffOffered(),
        active,
    };
}
