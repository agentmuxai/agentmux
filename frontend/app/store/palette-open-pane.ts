// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createBlock } from "@/app/store/block-layout-actions";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { isSingletonView } from "@/app/store/widget-singleton";

/**
 * Open the pane `blockdef` describes: a singleton view (the widget config's `singleton`) brings back
 * the one already open, anything else, or `forceNew`, opens a new one. The same rule as the widget bar.
 */
export function openPane(blockdef: BlockDef, forceNew = false): Promise<unknown> {
    const view = blockdef.meta?.["view"] as string | undefined;
    if (view && !forceNew && isSingletonView(view)) return openOrFocusPaneByView(view, blockdef);
    return createBlock(blockdef);
}
