// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The picture that follows the cursor during a pane drag, on Windows (drag/live-follow.ts, and
 * crates/cef/src/commands/live_follow.rs for the host half). A pane is torn off by dropping it outside every
 * AgentMux window (inside one the same drag is a tile drop), so the host shows the picture only there; it is
 * the pane's own picture (tearoff-snapshot.ts), held at the grab the pointer pressed it with, and the
 * floating window opens where it was last seen.
 */

import { getApi } from "@/store/global";
import { beginLiveFollow } from "./live-follow";
import { peekTearOffSnapshot } from "./tearoff-snapshot";

/** How long the drag waits for the pane's picture; without one the pane tears off as before. */
const PICTURE_WAIT_MS = 1500;

/** Hand the host the pane's picture, once its drag hook is armed. Never throws. */
export async function followPaneDrag(blockId: string, windowLabel: string): Promise<void> {
    const shot = await peekTearOffSnapshot(blockId, PICTURE_WAIT_MS);
    if (!shot) return;
    await beginLiveFollow(getApi(), {
        sourceWindowLabel: windowLabel,
        picture: shot.picture,
        windowWidth: shot.size.width,
        windowHeight: shot.size.height,
        offsetX: shot.grab.x,
        offsetY: shot.grab.y,
    });
}
