// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Terminal's contribution to the shared pane-tab model (pane-tab-model.tsx):
 * a user-set title (double-click rename, persisted as `pane-title`), else the
 * remote it runs on (pane-tab-remote.ts), else a position-based "Terminal N".
 * A remote terminal's icon is the remote's; a local one keeps the shared
 * default for `term`.
 */

import type { PaneTabDescriptor } from "@/element/pane-tab-model";
import { remoteOf, remoteTabIcon, remoteTabName } from "@/app/block/pane-tab-remote";
import { setBlockMeta } from "@/app/store/block-meta";

/** The terminal manifest's `tab` (block-registry.ts). */
export const termPaneTab: PaneTabDescriptor = {
    // `ordinal` counts only terminal members, so numbering stays contiguous
    // when another widget type sits between them in the stack.
    label: ({ meta, ordinal }) => {
        const title = meta?.["pane-title"] as string | undefined;
        if (title) return title;
        const remote = remoteOf(meta);
        return remote ? remoteTabName(remote) : `Terminal ${Math.max(ordinal, 1)}`;
    },
    icon: ({ meta }) => {
        const remote = remoteOf(meta);
        return remote ? remoteTabIcon(remote) : undefined;
    },
    renamer: ({ blockId }) => async (title: string) => {
        await setBlockMeta(blockId, { "pane-title": title } as any);
    },
};
