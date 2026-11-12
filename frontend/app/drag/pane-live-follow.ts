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
 * While a drop is armed, the picture is made smaller until it fits the rect the drop would give the pane (the
 * live reflow's landing rect, liveReflow.ts), so the pane in hand is never larger than the place it is going
 * (the operator, 2026-11-10: "the floater should change size with the size of the landing site"). It is the
 * whole pane, resized in its own proportions: cut to the rect, it lost its close buttons and half its content
 * (the operator, 2026-11-12: "we dont want the hard cutoff. Can it simply be resized instead of the edge cut
 * off?"). It is never made larger than the pane's own picture: one padded out to a larger landing rect was a
 * long blank band beside the pane, and the pane at its landing size is on screen anyway, in the landing rect
 * itself. With no drop armed it is the pane's own size again.
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

/** The smallest the pane in hand gets, as a share of its own size: smaller, and it is no longer a pane to the eye. */
export const MIN_FOLLOW_SCALE = 0.35;

/**
 * The size of the pane in hand for a landing rect: the pane's own picture made smaller until it fits the rect
 * on both axes, in its own proportions, never larger than itself and never smaller than `MIN_FOLLOW_SCALE`.
 */
export function followSize(own: { width: number; height: number }, landing?: { width: number; height: number }): { width: number; height: number } {
    const fit = landing ? Math.min(1, landing.width / own.width, landing.height / own.height) : 1;
    const scale = Math.max(MIN_FOLLOW_SCALE, fit);
    return { width: Math.round(own.width * scale), height: Math.round(own.height * scale) };
}

/** Resize the picture to the landing while it changes, from `own` (its size, and the most it shows), until the drag ends. */
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
