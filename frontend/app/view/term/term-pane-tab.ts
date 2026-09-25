// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Terminal's contribution to the shared pane-tab model (pane-tab-model.tsx):
 * a user-set title (double-click rename, persisted as `pane-title`) or a
 * position-based "Terminal N". The icon is the shared default for `term`.
 */

import { MOS } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { PaneTabDescriptor } from "@/element/pane-tab-model";

/** The terminal manifest's `tab` (block-registry.ts). */
export const termPaneTab: PaneTabDescriptor = {
    // `ordinal` counts only terminal members, so numbering stays contiguous
    // when another widget type sits between them in the stack.
    label: ({ meta, ordinal }) => (meta?.["pane-title"] as string | undefined) || `Terminal ${Math.max(ordinal, 1)}`,
    renamer: ({ blockId }) => async (title: string) => {
        await RpcApi.SetMetaCommand(TabRpcClient, {
            oref: MOS.makeORef("block", blockId),
            meta: { "pane-title": title } as any,
        });
    },
};
