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
 * easier to manage the pane, but the shape and aspect ratio should match the landing site"). With no drop
 * armed the picture is the pane's own size and shape again.
 *
 * A picture resized to another shape is a squashed pane (the operator, 2026-11-12: "we have the shape but not
 * the aspect ratio"). The pane laid out for the landing rect is on screen, in the rect itself, once the panes
 * have come to rest on the drop: the host takes its picture there (`liveFollowCapture`) and the pane in hand
 * becomes that picture, the pane as it will land. Until it is taken (a quarter of a second after a drop arms,
 * and never for a drop the pointer only passes) the first picture stands in, resized. A picture taken once is
 * kept for the drag, so a drop the pointer comes back to has its own at once.
 */

import { createEffect, createRoot, onCleanup, untrack } from "solid-js";
import { getApi } from "@/store/global";
import { beginLiveFollow, captureLiveFollow, resizeLiveFollow, type LandingCapture } from "./live-follow";
import { peekTearOffSnapshot } from "./tearoff-snapshot";

/** How long the drag waits for the pane's picture; without one the pane tears off as before. */
const PICTURE_WAIT_MS = 1500;

/** Where the drag's drop would put the pane, read reactively: the layout model's landing rect, while the drag lasts. */
export interface LandingSource {
    /** The landing rect's size, or none while no drop is armed. */
    rect: () => { width: number; height: number } | undefined;
    /** Whether the drag is still on; the picture stops following the landing when it ends. */
    active: () => boolean;
    /**
     * Set, and a new value each time, once the panes stand in the drop being shown; none before. Without it
     * the picture is never taken again.
     */
    settled?: () => unknown;
    /** The pane on screen in its landing rect, read now: where its picture is taken, and what is drawn over it. */
    site?: () => Pick<LandingCapture, "rect" | "viewportWidth" | "wash"> | undefined;
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

/**
 * The name of a landing rect, for the picture taken of the pane in it: its size. The pane is laid out the same
 * in two rects of one size, wherever they are.
 */
export function landingKey(rect: { width: number; height: number }): string {
    return `${Math.round(rect.width)}x${Math.round(rect.height)}`;
}

/** How many frames after the panes come to rest the pane's picture is taken: content that lays itself out a frame late has. */
const REST_FRAMES = 2;

/** Run `fn` after `frames` frames, or at once where there are none (a test). Returns what cancels it. */
function afterFrames(frames: number, fn: () => void): () => void {
    if (typeof requestAnimationFrame !== "function") {
        fn();
        return () => {};
    }
    let id = 0;
    const step = (left: number) => {
        id = requestAnimationFrame(() => (left > 1 ? step(left - 1) : fn()));
    };
    step(frames);
    return () => cancelAnimationFrame(id);
}

/**
 * Keep the picture to the landing, from `own` (the pane's size), until the drag ends: its size while the
 * landing changes, and the pane's own picture in the landing rect once the panes rest there.
 */
function followLanding(landing: LandingSource, own: { width: number; height: number }): void {
    createRoot((dispose) => {
        let shown = { width: Math.round(own.width), height: Math.round(own.height), key: undefined as string | undefined };
        createEffect(() => {
            const rect = landing.rect();
            if (!landing.active()) {
                dispose();
                return;
            }
            const next = { ...followSize(own, rect), key: rect && landingKey(rect) };
            if (next.width === shown.width && next.height === shown.height && next.key === shown.key) return;
            shown = next;
            const size = { width: next.width, height: next.height };
            void resizeLiveFollow(getApi(), size, next.key === undefined ? undefined : { landingKey: next.key });
        });
        createEffect(() => {
            if (!landing.settled?.() || !landing.active()) return;
            // The panes moved on, or the drag ended, before the picture was taken or kept: it is of nothing now.
            let rests = true;
            const cancel = afterFrames(REST_FRAMES, () => void retake(landing, own, () => rests));
            onCleanup(() => {
                rests = false;
                cancel();
            });
        });
    });
}

/** Take the pane's picture where it stands in its landing rect, and show it if the pane stood still meanwhile. */
async function retake(landing: LandingSource, own: { width: number; height: number }, rests: () => boolean): Promise<void> {
    const rect = untrack(landing.rect);
    const site = rect && landing.site?.();
    if (!rect || !site || !rests()) return;
    const key = landingKey(rect);
    const size = followSize(own, rect);
    const staged = await captureLiveFollow(getApi(), { landingKey: key, size, ...site }, rests);
    if (staged && rests()) void resizeLiveFollow(getApi(), size, { landingKey: key, adopt: true });
}
