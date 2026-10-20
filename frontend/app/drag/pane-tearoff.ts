// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Dragging a whole pane out of the window by its header (the `"tile"` drag)
 * tears off the pane with ALL of its tabs, not only the visible one.
 * Shared by the three per-platform `CrossWindowDragMonitor`s.
 *
 * A pane that holds several pane tabs is one layout leaf with a `blockStack`;
 * `node.data.blockId` is only the visible tab. One pane-tab pill dragged out
 * on its own is a different payload (`pane-tab-tearoff.ts`) and moves just
 * that tab.
 *
 * RCA_WHOLE_PANE_TEAROFF_MOVES_ONLY_THE_VISIBLE_TAB_2026_10_19.md
 */

import { removeMovedBlock } from "@/layout/lib/layoutMagnify";
import type { LayoutModel } from "@/layout/lib/layoutModel";
import { effectiveStack } from "@/layout/lib/stackMembers";
import type { LayoutNode } from "@/layout/lib/types";

/** What a header drag of a pane tears off: its visible tab and its whole tab
 *  stack in order (`[blockId]` for a pane with one tab). */
export interface PaneTearOff {
    blockId: string;
    members: string[];
}

/** The tear-off a `"tile"` drag of `node` stands for, or null when the node
 *  holds no block. */
export function paneTearOffOf(node: LayoutNode | undefined): PaneTearOff | null {
    const data = node?.data;
    const blockId = data?.blockId;
    if (!data || !blockId) return null;
    const stack = effectiveStack(data);
    return { blockId, members: stack.includes(blockId) ? [...stack] : [blockId] };
}

/** Take a torn-off pane's tabs out of the source layout once the floating
 *  window shows them. Each leaves its leaf (stack-safe), and the pane goes
 *  with the last; a no-op for a block the backend's queued `delete` already
 *  removed. */
export function removeTornOffPane(model: LayoutModel, members: string[]): void {
    for (const blockId of members) {
        removeMovedBlock(model, blockId);
    }
}
