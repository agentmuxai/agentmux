// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getLayoutModelForStaticTab } from "@/layout/lib/layoutModelHooks";
import { closeBlockInStack, effectiveStack } from "@/layout/lib/layoutStack";

/** Close `blockId`'s pane tab, but never the pane itself: false when it is
 *  the pane's only tab (the pane's own × closes that), or when the pane is not
 *  in the window tab on screen. */
export async function closeOwnTab(blockId: string): Promise<boolean> {
    const model = getLayoutModelForStaticTab();
    const node = model?.getNodeByBlockId(blockId);
    if (!model || !node?.data || effectiveStack(node.data).length <= 1) return false;
    await closeBlockInStack(model, node.id, blockId);
    return true;
}
