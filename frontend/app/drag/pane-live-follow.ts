// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The picture that follows the cursor during a pane drag, on Windows (drag/live-follow.ts, and
 * crates/cef/src/commands/live_follow.rs for the host half). It is the pane's own picture
 * (tearoff-snapshot.ts), held at the grab the pointer pressed it with, and it follows from the drag's first
 * move wherever the cursor goes: over the layout, where the drop is a tile drop and the other panes make room
 * under it, and outside every AgentMux window, where the drop tears the pane off and the floating window
 * opens where the picture was last seen. No translucent ghost ever stands in for the pane.
 *
 * While a drop is armed, the picture takes the size of the rect the drop would give the pane (the live
 * reflow's landing rect, liveReflow.ts), cut or padded, never scaled, so the pane in hand is the pane as it
 * will land (the operator, 2026-11-10: "the floater should change size with the size of the landing site");
 * with no drop armed it is the pane's own size again.
 */

import { createEffect, createRoot } from "solid-js";
import { getApi } from "@/store/global";
import { beginLiveFollow, resizeLiveFollow } from "./live-follow";
import { peekTearOffSnapshot } from "./tearoff-snapshot";

/** How long the drag waits for the pane's picture; without one the pane tears off as before. */
const PICTURE_WAIT_MS = 1500;

/** Where the drag's drop would put the pane, read reactively: the layout model's landing rect, while the drag lasts. */
export interface LandingSource {
    /** The landing rect's size, or none while no drop is armed. */
    rect: () => { width: number; height: number } | undefined;
    /** Whether the drag is still on; the picture stops following the landing when it ends. */
    active: () => boolean;
}

/** Hand the host the pane's picture, once its drag hook is armed, and keep its size to the landing. Never throws. */
export async function followPaneDrag(blockId: string, windowLabel: string, landing?: LandingSource): Promise<void> {
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
    if (landing) followLanding(landing, shot.size);
}

/** Size the picture to the landing while it changes, from `own` (its size now), until the drag ends. */
function followLanding(landing: LandingSource, own: { width: number; height: number }): void {
    createRoot((dispose) => {
        let shown = { width: Math.round(own.width), height: Math.round(own.height) };
        createEffect(() => {
            const rect = landing.rect();
            if (!landing.active()) {
                dispose();
                return;
            }
            const want = rect ?? own;
            const next = { width: Math.round(want.width), height: Math.round(want.height) };
            if (next.width === shown.width && next.height === shown.height) return;
            shown = next;
            void resizeLiveFollow(getApi(), next);
        });
    });
}
