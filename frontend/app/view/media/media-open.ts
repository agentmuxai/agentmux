// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Open a media file where the user is already looking at media: as a tab of
 * the Media pane on screen, rather than a new pane each time.
 * docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §5.7.
 */

import { setBlockMeta } from "@/app/store/block-meta";
import { getObjectValue, makeORef } from "@/app/store/mos";
import { getLayoutModelForStaticTab } from "@/layout/lib/layoutModelHooks";
import { META_OPEN, type MediaOpenRequest } from "./media-pane";

/** A Media pane shown in the window tab on screen: the focused one if it is
 *  one, else the first. */
export function mediaPaneOnScreen(): string | null {
    const model = getLayoutModelForStaticTab();
    if (!model) return null;
    const isMedia = (id: string | undefined): id is string =>
        !!id && getObjectValue<Block>(makeORef("block", id))?.meta?.view === "media";
    const focused = model.focusedNode()?.data?.blockId;
    if (isMedia(focused)) return focused;
    for (const leaf of model.leafs()) {
        const id = leaf.data?.blockId;
        if (isMedia(id)) return id;
    }
    return null;
}

/**
 * Add `path` as a tab of the Media pane on screen. False when there is none
 * (the caller opens a new pane).
 */
export async function openInMediaPaneOnScreen(path: string): Promise<boolean> {
    const blockId = mediaPaneOnScreen();
    if (!blockId) return false;
    const queued = getObjectValue<Block>(makeORef("block", blockId))?.meta?.[META_OPEN];
    const list: unknown[] = Array.isArray(queued) ? queued : [];
    const request: MediaOpenRequest = { id: crypto.randomUUID(), path };
    await setBlockMeta(blockId, { [META_OPEN]: [...list, request] } as MetaType);
    return true;
}
