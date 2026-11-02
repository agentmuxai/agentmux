// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane-tab pill that lives in another window tab, dropped on a pane of this one (the view switched to this
 * tab while the pill was dragged over its button). It is in no pane of this layout, so the pane's own
 * stack moves cannot place it; it goes through the one cross-tab move instead.
 * docs/specs/SPEC_PANE_DRAG_TO_TAB_2026_07_10.md.
 */

import { moveBlocksToTab, tabOfBlock } from "@/layout/index";
import type { LayoutModel } from "@/layout/lib/layoutModel";
import { findNodeByBlockId } from "@/layout/lib/layoutNode";
import type { DropDirection } from "@/layout/lib/types";

/** How the pill lands: as a tab of the pane it was dropped on, or as a pane beside it. */
export type ForeignLanding = { asTab: true } | { direction: DropDirection };

export function foreignTabMoves(layoutModel: LayoutModel, paneBlockId: () => string | undefined) {
    return {
        /** Whether `blockId` is in no pane of this window tab's layout. */
        isForeign(blockId: string): boolean {
            const root = layoutModel.treeState.rootNode;
            return !!root && !findNodeByBlockId(root, blockId);
        },
        /** Moves `blockIds` (the grabbed one visible) next to the pane. Always false: the landing shows when the layout update arrives, not as a bounce of the pill. */
        move(blockIds: readonly string[], grabbed: string, landing: ForeignLanding): false {
            const sourceTabId = tabOfBlock(blockIds[0]);
            const targetTabId = layoutModel.tabAtom()?.oid;
            const targetBlockId = paneBlockId();
            if (sourceTabId && targetTabId && targetBlockId) {
                moveBlocksToTab({ blockIds, activeBlockId: grabbed, sourceTabId, targetTabId, targetBlockId, ...landing });
            }
            return false;
        },
    };
}
