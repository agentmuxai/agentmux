// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The picture that follows the cursor during a pane drag, on Windows (drag/live-follow.ts, and
 * crates/cef/src/commands/live_follow.rs for the host half). It is the pane's own picture
 * (tearoff-snapshot.ts), held at the grab the pointer pressed it with, and it follows from the drag's first
 * move wherever the cursor goes: over the layout, where the drop is a tile drop and the other panes make room
 * under it, and outside every AgentMux window, where the drop tears the pane off and the floating window
 * opens where the picture was last seen. No translucent ghost ever stands in for the pane. The picture stays
 * on the monitor its window is on: it stops whole at that monitor's edge, as a window does, while the cursor
 * goes on (the host's `home`, live_follow.rs).
 *
 * While a drop is armed, the picture takes the shape of the rect the drop would give the pane (the live
 * reflow's landing rect, liveReflow.ts), smaller than it (`followSize`), so the pane in hand looks like what is
 * about to land (the operator, 2026-11-10: "the floater should change size with the size of the landing
 * site"). It is the whole pane, resized: cut to the rect, it lost its close buttons and half its content (the
 * operator, 2026-11-12: "we dont want the hard cutoff. Can it simply be resized instead of the edge cut off?").
 * Resized in the pane's own proportions it was a long pane held over an upright landing site, so it is resized
 * on each axis to the site's proportions (the operator, 2026-11-12: "We want some shrinkage, which makes it
 * easier to manage the pane, but the shape and aspect ratio should match the landing site"). The pane as it
 * will really be laid out is on screen anyway, in the landing rect itself. With no drop armed the picture is
 * the pane's own size and shape again.
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

/** How much smaller than its landing rect (or than itself, if that is smaller) the pane in hand is, on each axis. */
export const HAND_SCALE = 0.7;

/**
 * The size of the pane in hand for a landing rect: the rect's own shape, at `HAND_SCALE` of the smaller of the
 * two, the landing rect and the pane (by area). So it is always smaller than the place it is going, by the
 * same share on both axes, and a small pane headed for a large place is not blown up to it. With no landing
 * rect it is the pane's own size.
 */
export function followSize(own: { width: number; height: number }, landing?: { width: number; height: number }): { width: number; height: number } {
    if (!landing || !(landing.width > 0) || !(landing.height > 0)) {
        return { width: Math.round(own.width), height: Math.round(own.height) };
    }
    const area = Math.min(own.width * own.height, landing.width * landing.height) * HAND_SCALE * HAND_SCALE;
    const shape = landing.width / landing.height;
    return { width: Math.round(Math.sqrt(area * shape)), height: Math.round(Math.sqrt(area / shape)) };
}

/** Resize the picture to the landing while it changes, from `own` (the pane's size), until the drag ends. */
function followLanding(landing: LandingSource, own: { width: number; height: number }): void {
    createRoot((dispose) => {
        let shown = { width: Math.round(own.width), height: Math.round(own.height) };
        createEffect(() => {
            const rect = landing.rect();
            if (!landing.active()) {
                dispose();
                return;
            }
            const next = followSize(own, rect);
            if (next.width === shown.width && next.height === shown.height) return;
            shown = next;
            void resizeLiveFollow(getApi(), next);
        });
    });
}
