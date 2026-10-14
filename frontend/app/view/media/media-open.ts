// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Open a media file where the user is already looking at media: as a tab of
 * the Media pane on screen, rather than a new pane each time.
 * docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §5.7; the pane is shown too:
 * docs/specs/SPEC_MEDIA_OPEN_SHOWS_THE_MEDIA_PANE_2026_10_12.md.
 */

import { setBlockMeta } from "@/app/store/block-meta";
import { getObjectValue, makeORef } from "@/app/store/mos";
import { showBlockWithoutFocus } from "@/app/util/reveal-block";
import { getLayoutModelForStaticTab } from "@/layout/lib/layoutModelHooks";
import { effectiveStack } from "@/layout/lib/stackMembers";
import { META_OPEN, type MediaOpenRequest } from "./media-pane";

/**
 * The Media pane to add a file to, among the panes of the window tab on screen:
 * the focused one if it is a Media pane, else the first Media pane the user can
 * already see (not minimized, and the front tab of its stack), else the first
 * one of any kind (the open then shows it). A Media pane that is a background
 * tab of a stack counts, so a second one is not opened.
 * SPEC_MEDIA_OPEN_SHOWS_THE_MEDIA_PANE_2026_10_12.md §3.1, §4.1.
 */
function mediaPaneOnScreen(): string | null {
    const model = getLayoutModelForStaticTab();
    if (!model) return null;
    const isMedia = (id: string | undefined): id is string =>
        !!id && getObjectValue<Block>(makeORef("block", id))?.meta?.view === "media";
    const focusedData = model.focusedNode()?.data;
    const focused = focusedData?.activeBlockId || focusedData?.blockId;
    if (isMedia(focused)) return focused;
    let firstAny: string | null = null;
    for (const leaf of model.leafs()) {
        const data = leaf.data;
        if (!data) continue;
        const front = data.activeBlockId || data.blockId;
        for (const id of effectiveStack(data)) {
            if (!isMedia(id)) continue;
            if (!leaf.minimized && id === front) return id; // already visible: nothing to restore
            firstAny ??= id;
        }
    }
    return firstAny;
}

/**
 * Add `path` as a tab of the Media pane on screen, and show that pane. False
 * when there is none (the caller opens a new pane).
 */
export async function openInMediaPaneOnScreen(path: string): Promise<boolean> {
    const blockId = mediaPaneOnScreen();
    if (!blockId) return false;
    const queued = getObjectValue<Block>(makeORef("block", blockId))?.meta?.[META_OPEN];
    const list: unknown[] = Array.isArray(queued) ? queued : [];
    const request: MediaOpenRequest = { id: crypto.randomUUID(), path };
    await setBlockMeta(blockId, { [META_OPEN]: [...list, request] } as MetaType);
    // Show the pane the file landed in: restore it if it is minimized, end another
    // pane's maximize, switch a stack to it. No window-tab switch, no focus or caret
    // move. The file is queued either way, so a failure here is not the click's.
    await showBlockWithoutFocus(blockId).catch(() => false);
    return true;
}
