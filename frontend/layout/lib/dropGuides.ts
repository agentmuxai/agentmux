// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Drop guides: the places a drag can be released over a pane, drawn as hot spots, so a drop is chosen by
 * hovering its spot rather than by hunting for the right part of the pane (the operator, 2026-11-14: "we want
 * to provide guides, like hot spots where if you hover you get the effect you want. currently the experience
 * is you are dragging around randomly to get a position").
 *
 * A spot is one outcome (dropOptions.ts): a layout the release leaves that no other spot of the pane leaves,
 * and that is not the layout as it is. Its picture is that layout, drawn small, with the dragged pane filled
 * in, so what a spot shows is what hovering it shows and what releasing on it makes. The spots sit in a
 * compass in the middle of the hovered pane, each on the side its drop is named for; a drop beside the pane's
 * whole row or column, where that is another layout, has its spot at the pane's edge. Over a spot, its drop
 * is shown; anywhere else in the pane, nothing is (a tab pill joins the pane's tabs there, as it always has,
 * and the middle spot says so).
 *
 * The spots are laid out on the layout's own rects, which no drag changes (dropOptions.ts `layoutShape`), so
 * they stay where they are while the live reflow moves the panes under them.
 */

import { createSignal } from "solid-js";
import type { DropOption } from "./dropOptions";
import type { LayoutModel } from "./layoutModel";
import { DropDirection } from "./types";

/** A pane of a spot's picture, as fractions of the layout. */
export interface GuidePane {
    left: number;
    top: number;
    width: number;
    height: number;
    /** The dragged pane, where the drop leaves it. */
    moved: boolean;
    /** The hovered pane of a join: the tab goes into its tabs. */
    joined: boolean;
}

export interface GuideSpot {
    direction: DropDirection;
    /** The spot's centre, in the layout container's coordinates. */
    x: number;
    y: number;
    width: number;
    height: number;
    /** The layout the drop leaves. */
    panes: GuidePane[];
    /** The tab joins the pane's tabs: what a release anywhere else in the pane does too. */
    join: boolean;
}

export interface GuideShown {
    targetId: string;
    spots: GuideSpot[];
    /** The spot whose drop is shown, or would be made by a release now. */
    active?: DropDirection;
}

/** The widest spot, px. Smaller panes get smaller spots. */
const SPOT_W = 56;
/** The narrowest spot: a pane too small for the compass at this size has it overflow rather than vanish. */
const SPOT_W_MIN = 28;
/** Gap between two spots of the compass, px. */
const SPOT_GAP = 5;
/** How far past its edge a spot still takes the pointer, px. */
const SPOT_SLOP = 5;
/**
 * How long a guide stays after the drop it was shown with is taken down, ms: crossing from one pane's zones to
 * the next, or a tab pill handed between a pane and the layout, is a clear and then a sample a frame later.
 */
export const GUIDE_GRACE_MS = 120;
/** How far an edge spot stands in from the pane's edge, px. */
const EDGE_INSET = 12;

/** The layout's extent, from the rects of its panes. */
function boundsOf(rects: Dimensions[]): { width: number; height: number } {
    let [width, height] = [1, 1];
    for (const r of rects) {
        width = Math.max(width, r.left + r.width);
        height = Math.max(height, r.top + r.height);
    }
    return { width, height };
}

function picture(option: DropOption, targetId: string, now: { id: string; rect: Dimensions }[], bounds: { width: number; height: number }): GuidePane[] {
    const pane = (rect: Dimensions, moved: boolean, joined: boolean): GuidePane => ({
        left: rect.left / bounds.width,
        top: rect.top / bounds.height,
        width: rect.width / bounds.width,
        height: rect.height / bounds.height,
        moved,
        joined,
    });
    const future = option.future;
    if (!future) return now.map((p) => pane(p.rect, false, !!option.join && p.id === targetId));
    return future.leafIds.filter((id) => future.rects[id]).map((id) => pane(future.rects[id], id === future.movedId, false));
}

/** Where each direction's spot sits, in steps of one spot from the middle of the pane. */
const COMPASS: Partial<Record<DropDirection, [number, number]>> = {
    [DropDirection.Center]: [0, 0],
    [DropDirection.Top]: [0, -1],
    [DropDirection.Right]: [1, 0],
    [DropDirection.Bottom]: [0, 1],
    [DropDirection.Left]: [-1, 0],
};

/**
 * The spots for the drops `options` over the pane at `rect`. `now` is every pane of the layout as it stands.
 * A drop beside the pane's whole group stands at the pane's edge where there is room clear of the compass,
 * and next to the compass's outer spot otherwise.
 */
export function guideSpots(rect: Dimensions, targetId: string, options: DropOption[], now: { id: string; rect: Dimensions }[]): GuideSpot[] {
    const bounds = boundsOf(now.map((p) => p.rect));
    // A spot has the layout's own proportions, within reason: a picture of it has to fit.
    const ratio = Math.min(1, Math.max(0.5, bounds.height / bounds.width));
    const fit = Math.min((rect.width - 2 * SPOT_GAP - 16) / 3, (rect.height - 2 * SPOT_GAP - 16) / 3 / ratio);
    const width = Math.round(Math.max(SPOT_W_MIN, Math.min(SPOT_W, fit)));
    const height = Math.round(width * ratio);
    const [stepX, stepY] = [width + SPOT_GAP, height + SPOT_GAP];
    const [cx, cy] = [rect.left + rect.width / 2, rect.top + rect.height / 2];
    const edgeX = rect.width / 2 - EDGE_INSET - width / 2;
    const edgeY = rect.height / 2 - EDGE_INSET - height / 2;
    // Clear of the compass: past its outer spot, with a gap a pointer can rest in.
    const outerX = edgeX >= 2 * stepX + SPOT_GAP ? edgeX : 2 * stepX;
    const outerY = edgeY >= 2 * stepY + SPOT_GAP ? edgeY : 2 * stepY;
    const place = (direction: DropDirection): [number, number] => {
        const at = COMPASS[direction];
        if (at) return [cx + at[0] * stepX, cy + at[1] * stepY];
        switch (direction) {
            case DropDirection.OuterTop:
                return [cx, cy - outerY];
            case DropDirection.OuterBottom:
                return [cx, cy + outerY];
            case DropDirection.OuterLeft:
                return [cx - outerX, cy];
            default:
                return [cx + outerX, cy];
        }
    };
    return options.map((option) => {
        const [x, y] = place(option.direction);
        return { direction: option.direction, x, y, width, height, panes: picture(option, targetId, now, bounds), join: !!option.join };
    });
}

/** The spot to light for a drop in `direction`: its own, or the join's where a release with no drop joins. */
export function activeFor(spots: GuideSpot[], direction: DropDirection | undefined): DropDirection | undefined {
    if (direction !== undefined && spots.some((s) => s.direction === direction)) return direction;
    return spots.find((s) => s.join)?.direction;
}

/** The spot under `offset` (container coordinates), if any. */
export function spotAt(spots: GuideSpot[], offset: { x: number; y: number }): GuideSpot | undefined {
    for (const spot of spots) {
        if (Math.abs(offset.x - spot.x) <= spot.width / 2 + SPOT_SLOP && Math.abs(offset.y - spot.y) <= spot.height / 2 + SPOT_SLOP) return spot;
    }
    return undefined;
}

// The spots of a pane, made once for the drops they stand for: the same drops (dropOptions.ts gives the same
// array back for the same pane of the same layout) are the same spots, so their elements stay on screen and
// only the active one changes.
const spotsOf = new WeakMap<DropOption[], { key: string; spots: GuideSpot[] }>();

export function guideSpotsFor(model: LayoutModel, rect: Dimensions, targetId: string, options: DropOption[]): GuideSpot[] {
    const key = `${targetId}:${rect.left},${rect.top},${rect.width},${rect.height}`;
    const held = spotsOf.get(options);
    if (held?.key === key) return held.spots;
    const now = model.leafs().flatMap((leaf) => {
        const r = model.getNodeRectById(leaf.id);
        return r ? [{ id: leaf.id, rect: r }] : [];
    });
    const spots = guideSpots(rect, targetId, options, now);
    spotsOf.set(options, { key, spots });
    return spots;
}

const sameGuide = (a: GuideShown | undefined, b: GuideShown | undefined): boolean =>
    a === b || (!!a && !!b && a.targetId === b.targetId && a.active === b.active && a.spots === b.spots);

const shown = new WeakMap<LayoutModel, ReturnType<typeof createSignal<GuideShown | undefined>>>();
const hideTimers = new WeakMap<LayoutModel, ReturnType<typeof setTimeout>>();
/** The layouts showing a guide, so the end of a drag takes every one down. */
const guided = new Set<LayoutModel>();

/**
 * Told when the first guide of any layout shows (true) and when the last one goes (false, after its grace): the
 * picture the cursor carries is over the guides, and fades while there is one (placementPreview.ts).
 */
let guidesShown: (shown: boolean) => void = () => {};
export function onGuidesShown(listener: (shown: boolean) => void): void {
    guidesShown = listener;
}

function signalOf(model: LayoutModel) {
    let s = shown.get(model);
    if (!s) {
        s = createSignal<GuideShown | undefined>(undefined, { equals: sameGuide });
        shown.set(model, s);
    }
    return s;
}

/** The guide shown over `model`'s layout, for the guide layer (DropGuideLayer.tsx). Reactive. */
export function guideIn(model: LayoutModel): GuideShown | undefined {
    return signalOf(model)[0]();
}

/** Show the guide for one sample of a drag over a pane. */
export function showGuide(model: LayoutModel, guide: GuideShown): void {
    clearTimeout(hideTimers.get(model));
    hideTimers.delete(model);
    if (guided.size === 0) guidesShown(true);
    guided.add(model);
    signalOf(model)[1](guide);
}

/**
 * The drop that is shown for the pane `targetId` is `direction`'s: its spot is the active one. The shown drop
 * is the steadied one (dropIntentTracker.ts), which can trail the spot under the pointer for a moment.
 */
export function markGuideActive(model: LayoutModel, targetId: string, direction: DropDirection | undefined): void {
    const [get, set] = signalOf(model);
    const guide = get();
    if (!guide || guide.targetId !== targetId) return;
    const active = activeFor(guide.spots, direction);
    if (active !== guide.active) set({ ...guide, active });
}

/**
 * Take the guide down, after `graceMs` with no sample showing one again: crossing from one pane's zones to the
 * next, or a tab pill handed between a pane and the layout, is a clear and then a sample a frame later.
 */
export function hideGuide(model: LayoutModel, graceMs = 0): void {
    clearTimeout(hideTimers.get(model));
    const hide = () => {
        hideTimers.delete(model);
        if (guided.delete(model) && guided.size === 0) guidesShown(false);
        signalOf(model)[1](undefined);
    };
    if (graceMs <= 0) hide();
    else hideTimers.set(model, setTimeout(hide, graceMs));
}

/** The drag is over: no guide stays, in any layout. */
export function hideAllGuides(): void {
    for (const model of [...guided]) hideGuide(model);
}
