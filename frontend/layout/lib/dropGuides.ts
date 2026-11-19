// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Drop guides: the places a drag can be released over a pane, drawn as hot spots, so a drop is chosen by
 * hovering its spot rather than by hunting for the right part of the pane, not by dragging around until the
 * pane takes a position.
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

/**
 * A spot's width is this share of the hovered pane (its width, or its height over the picture's proportions,
 * whichever is smaller), so the guides grow with the pane they are over: a large pane has large guides and a
 * small one has small ones. Three across, the compass is then about half of the pane's width and less of its
 * height, which leaves the panes under it, as the live reflow moves them, in view. A small pane keeps the size every
 * pane had before ([`SPOT_W_BASE`]), as far as it fits in it, so its guides are never smaller than they were.
 */
const SPOT_SHARE = 0.15;
/** The size of a spot before it followed the pane: where the share of a small pane is less, this is the size. */
const SPOT_W_BASE = 56;
/** The widest spot, px: on a very large pane the compass stays one a pointer can take in at a glance. */
const SPOT_W_MAX = 180;
/** The narrowest spot: a pane too small for the compass at this size has it overflow rather than vanish. */
const SPOT_W_MIN = 28;
/** Gap between two spots of the compass: at least this many px (the gap of the narrowest spots)... */
const SPOT_GAP_MIN = 5;
/** ...and this share of a spot, so the gap keeps its look beside a larger spot. */
const SPOT_GAP_SHARE = 0.09;
/** How far an edge spot stands in from the pane's edge: at least this many px, and this share of a spot. */
const EDGE_INSET_MIN = 12;
const EDGE_INSET_SHARE = 0.21;
/** A spot's gap, px. A pointer also takes a spot this far past its edge (spotAt). */
const gapOf = (spot: number): number => Math.max(SPOT_GAP_MIN, Math.round(spot * SPOT_GAP_SHARE));
const insetOf = (spot: number): number => Math.max(EDGE_INSET_MIN, Math.round(spot * EDGE_INSET_SHARE));
/**
 * How long a guide stays after the drop it was shown with is taken down, ms: crossing from one pane's zones to
 * the next, or a tab pill handed between a pane and the layout, is a clear and then a sample a frame later.
 */
export const GUIDE_GRACE_MS = 120;
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

/** The spots of the compass side by side, and with a spot beside it on each side. */
const [COMPASS_ACROSS, WITH_OUTER_ACROSS] = [3, 5];
/** The room the spots leave to the pane's edges, both together, px. */
const PANE_MARGIN = 16;

/**
 * The widest spot (its width) that leaves `across` of them, with their gaps, inside a pane `extent` long, where
 * the spot is `scale` times its width along that extent (1 across, the picture's proportions down).
 */
function fitOf(extent: number, across: number, scale: number): number {
    const room = extent - PANE_MARGIN;
    // The narrowest gap if the spot is small enough to keep it, otherwise the gap that is a share of the spot.
    const narrow = (room - (across - 1) * SPOT_GAP_MIN) / (across * scale);
    return narrow * SPOT_GAP_SHARE <= SPOT_GAP_MIN ? narrow : room / (across * scale + (across - 1) * SPOT_GAP_SHARE);
}

/**
 * How far from the pane's middle a spot of `spot` along `extent` at its edge stands, or none where that is not
 * clear of the compass. `gap` and `inset` are those of the spots' width.
 */
function edgeOf(extent: number, spot: number, gap: number, inset: number): number | undefined {
    const edge = extent / 2 - inset - spot / 2;
    // Clear of the compass: past its outer spot, with a gap a pointer can rest in.
    return edge >= 2 * (spot + gap) + gap ? edge : undefined;
}

const isOuterX = (d: DropDirection): boolean => d === DropDirection.OuterLeft || d === DropDirection.OuterRight;
const isOuterY = (d: DropDirection): boolean => d === DropDirection.OuterTop || d === DropDirection.OuterBottom;

/**
 * The spots for the drops `options` over the pane at `rect`. `now` is every pane of the layout as it stands.
 * A drop beside the pane's whole group stands at the pane's edge where there is room clear of the compass,
 * and next to the compass's outer spot otherwise. Every spot is inside the pane: a pointer on a part of a spot
 * that is over the next pane is over that pane, whose guide takes this one's place, so that part can not be
 * chosen (found on macOS, 2026-11-15: a 174 px pane had its edge spot almost wholly over its neighbour). So
 * the spots are sized for as many as stand side by side, and a pane too small for five of the smallest has no
 * spot beside the compass on that axis: that drop is made over a larger pane of the same row or column.
 */
export function guideSpots(rect: Dimensions, targetId: string, options: DropOption[], now: { id: string; rect: Dimensions }[]): GuideSpot[] {
    const bounds = boundsOf(now.map((p) => p.rect));
    // A spot has the layout's own proportions, within reason: a picture of it has to fit.
    const ratio = Math.min(1, Math.max(0.5, bounds.height / bounds.width));
    const sized = (acrossX: number, acrossY: number): [number, number] => {
        const fit = Math.min(fitOf(rect.width, acrossX, 1), fitOf(rect.height, acrossY, ratio));
        // As wide as the pane's size asks, and no wider than the spots standing side by side fit in it.
        const wanted = Math.min(SPOT_W_MAX, Math.max(SPOT_W_BASE, SPOT_SHARE * Math.min(rect.width, rect.height / ratio)));
        const w = Math.round(Math.max(SPOT_W_MIN, Math.min(wanted, fit)));
        return [w, Math.round(w * ratio)];
    };
    let [width, height] = sized(COMPASS_ACROSS, COMPASS_ACROSS);
    // A spot beside the compass where the pane's edge is not clear of it: five side by side on that axis.
    const edge = (extent: number, spot: number) => edgeOf(extent, spot, gapOf(width), insetOf(width));
    const acrossX = options.some((o) => isOuterX(o.direction)) && edge(rect.width, width) === undefined ? WITH_OUTER_ACROSS : COMPASS_ACROSS;
    const acrossY = options.some((o) => isOuterY(o.direction)) && edge(rect.height, height) === undefined ? WITH_OUTER_ACROSS : COMPASS_ACROSS;
    if (acrossX !== COMPASS_ACROSS || acrossY !== COMPASS_ACROSS) [width, height] = sized(acrossX, acrossY);
    const gap = gapOf(width);
    const [stepX, stepY] = [width + gap, height + gap];
    const [cx, cy] = [rect.left + rect.width / 2, rect.top + rect.height / 2];
    const outerX = edge(rect.width, width) ?? 2 * stepX;
    const outerY = edge(rect.height, height) ?? 2 * stepY;
    // Five of the smallest spots are still wider than the pane: none beside the compass.
    const inside = (direction: DropDirection): boolean =>
        !(isOuterX(direction) && outerX + width / 2 > rect.width / 2) && !(isOuterY(direction) && outerY + height / 2 > rect.height / 2);
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
    const spots = options.filter((option) => inside(option.direction)).map((option) => {
        const [x, y] = place(option.direction);
        return { direction: option.direction, x, y, width, height, panes: picture(option, targetId, now, bounds), join: !!option.join };
    });
    return clearOfWide(spots, rect, bounds);
}

/** A rectangle of the layout, in the layout container's coordinates. */
interface Box {
    left: number;
    right: number;
    top: number;
    bottom: number;
}

const overlaps = (a: Box, b: Box): boolean => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

/** What a pointer takes as the spot: its rectangle and the slop past it (`spotAt`). */
const boxOfSpot = (s: GuideSpot): Box => {
    const [dx, dy] = [s.width / 2 + gapOf(s.width), s.height / 2 + gapOf(s.width)];
    return { left: s.x - dx, right: s.x + dx, top: s.y - dy, bottom: s.y + dy };
};

const boxOfSpots = (spots: GuideSpot[]): Box => {
    const boxes = spots.map(boxOfSpot);
    return {
        left: Math.min(...boxes.map((b) => b.left)),
        right: Math.max(...boxes.map((b) => b.right)),
        top: Math.min(...boxes.map((b) => b.top)),
        bottom: Math.max(...boxes.map((b) => b.bottom)),
    };
};

/** Where the wide set stands when it shows, at the layout's top centre and its bottom centre: three spots side by side, and their slop. */
function wideBoxes(bounds: { width: number; height: number }): { top: Box; bottom: Box } | undefined {
    const size = wideSize(bounds);
    if (!size) return undefined;
    const half = 1.5 * size.width + size.gap + size.gap;
    const [left, right] = [bounds.width / 2 - half, bounds.width / 2 + half];
    const depth = size.inset + size.height + size.gap;
    return { top: { left, right, top: 0, bottom: depth }, bottom: { left, right, top: bounds.height - depth, bottom: bounds.height } };
}

const shiftSpots = (spots: GuideSpot[], dx: number, dy: number): GuideSpot[] => spots.map((s) => ({ ...s, x: s.x + dx, y: s.y + dy }));

/**
 * The compass of a pane that stands where the wide set shows (the pane's own guide and the wide one both show
 * there: the pointer is in the zone, the wide set appears, and the pane's guide must not be in its way). The rule:
 *
 *  1. Where the compass is clear of the wide set's places, it stays where it is.
 *  2. Otherwise it moves along the pane, away from the layout's edge the wide set is at: down from the top set, up from the bottom one,
 *     by as much as it takes to be clear, if it still is in the pane.
 *  3. A pane too short for that holds it beside the wide set instead: to the side the pane has room on, the nearer one when both have.
 *  4. A pane too small for either keeps it where it is, and the spots that are under the wide set while it shows go (`withWideSpots`).
 *
 * It is by the pane and the layout only, never by the pointer, so a pointer that moves from the zone to the compass does not
 * move the compass under it. The spots beside the compass (at the pane's edges) stay at the edges.
 */
function clearOfWide(spots: GuideSpot[], rect: Dimensions, bounds: { width: number; height: number }): GuideSpot[] {
    const wide = wideBoxes(bounds);
    const compass = spots.filter((s) => COMPASS[s.direction]);
    if (!wide || compass.length === 0) return spots;
    const [top, bottom] = [wide.top, wide.bottom];
    const at = boxOfSpots(compass);
    const hit = [top, bottom].filter((b) => overlaps(at, b));
    if (hit.length === 0) return spots;
    const pane: Box = { left: rect.left, right: rect.left + rect.width, top: rect.top, bottom: rect.top + rect.height };
    // The way along the pane first, then beside the wide set (the nearer side first).
    const tries: [number, number][] = [];
    for (const b of hit) tries.push(b === top ? [0, top.bottom - at.top] : [0, bottom.top - at.bottom]);
    for (const dx of [wide.top.left - at.right, wide.top.right - at.left].sort((a, b) => Math.abs(a) - Math.abs(b))) tries.push([dx, 0]);
    for (const [dx, dy] of tries) {
        const moved: Box = { left: at.left + dx, right: at.right + dx, top: at.top + dy, bottom: at.bottom + dy };
        // In the pane along the way it moved: a pane too short for the compass at all is held to its width only.
        const inside = dx !== 0 ? moved.left >= pane.left && moved.right <= pane.right : moved.top >= pane.top && moved.bottom <= pane.bottom;
        if (inside && !overlaps(moved, top) && !overlaps(moved, bottom)) {
            return spots.map((s) => (COMPASS[s.direction] ? shiftSpots([s], dx, dy)[0] : s));
        }
    }
    return spots;
}

/**
 * The pane's own spots with the wide ones: both show while the pointer is in the wide zone, and each is hit-tested.
 * A spot of the pane that is under a wide one (the last case of `clearOfWide`) is the one that goes. The same arrays
 * give the same one back, so the guide's elements stay on screen while the pointer moves.
 */
const mixed = new WeakMap<GuideSpot[], { wide: GuideSpot[]; spots: GuideSpot[] }>();
export function withWideSpots(own: GuideSpot[], wide: GuideSpot[]): GuideSpot[] {
    const held = mixed.get(own);
    if (held?.wide === wide) return held.spots;
    const under = wide.map(boxOfSpot);
    const spots = [...wide, ...own.filter((s) => !under.some((b) => overlaps(boxOfSpot(s), b)))];
    mixed.set(own, { wide, spots });
    return spots;
}

/**
 * The wide guides: a second set of spots, at the top centre and the bottom centre of the layout, for the drops that make
 * the dragged pane a band across the whole layout (across its top, its middle or its bottom), whichever pane the pointer
 * is over. Sized by the layout as the pane guides are by their pane: a share of it, within the floor and ceiling of a spot.
 */
const WIDE_SHARE = 0.09;

/** The size of the wide set for a layout `bounds` big: its spots, and the zone it shows in; none where the layout is too small for it. */
function wideSize(bounds: { width: number; height: number }) {
    const ratio = Math.min(1, Math.max(0.5, bounds.height / bounds.width));
    const width = Math.round(Math.max(SPOT_W_MIN, Math.min(SPOT_W_MAX, WIDE_SHARE * Math.min(bounds.width, bounds.height / ratio))));
    const height = Math.round(width * ratio);
    const [gap, inset] = [gapOf(width), insetOf(width)];
    // The three spots stand side by side, and a zone is the room above (or below) them: the set leaves most of the layout to its panes.
    const zoneHeight = inset + 2 * height;
    if (3 * width + 2 * gap > 0.6 * bounds.width || 2 * zoneHeight > 0.45 * bounds.height) return undefined;
    return { width, height, gap, inset, zoneHeight, halfWidth: 2 * width + gap };
}

/** The panes of the layout as it stands, in the layout container coordinates. */
function paneRects(model: LayoutModel): { id: string; rect: Dimensions }[] {
    return model.leafs().flatMap((leaf) => {
        const r = model.getNodeRectById(leaf.id);
        return r ? [{ id: leaf.id, rect: r }] : [];
    });
}

/** The zone of the layout the pointer is in where the wide guides show: its top centre or its bottom centre. */
export function wideZoneAt(model: LayoutModel, offset: { x: number; y: number }): "top" | "bottom" | undefined {
    const bounds = boundsOf(paneRects(model).map((p) => p.rect));
    const size = wideSize(bounds);
    if (!size || Math.abs(offset.x - bounds.width / 2) > size.halfWidth) return undefined;
    if (offset.y >= 0 && offset.y <= size.zoneHeight) return "top";
    if (offset.y <= bounds.height && offset.y >= bounds.height - size.zoneHeight) return "bottom";
    return undefined;
}

/**
 * The spots of the wide set in `zone` for the drops `options`: side by side in the order of the options (across the top,
 * the middle, the bottom), in the middle of the layout width and at its top or its bottom edge. Each is the layout the
 * drop leaves, drawn small.
 */
export function wideGuideSpots(zone: "top" | "bottom", targetId: string, options: DropOption[], now: { id: string; rect: Dimensions }[]): GuideSpot[] {
    const bounds = boundsOf(now.map((p) => p.rect));
    const size = wideSize(bounds);
    if (!size) return [];
    const y = zone === "top" ? size.inset + size.height / 2 : bounds.height - size.inset - size.height / 2;
    const middle = (options.length - 1) / 2;
    return options.map((option, i) => ({
        direction: option.direction,
        x: bounds.width / 2 + (i - middle) * (size.width + size.gap),
        y,
        width: size.width,
        height: size.height,
        panes: picture(option, targetId, now, bounds),
        join: false,
    }));
}

/** The spot to light for a drop in `direction`: its own, or the join's where a release with no drop joins. */
export function activeFor(spots: GuideSpot[], direction: DropDirection | undefined): DropDirection | undefined {
    if (direction !== undefined && spots.some((s) => s.direction === direction)) return direction;
    return spots.find((s) => s.join)?.direction;
}

/** The spot under `offset` (container coordinates), if any. */
export function spotAt(spots: GuideSpot[], offset: { x: number; y: number }): GuideSpot | undefined {
    for (const spot of spots) {
        const slop = gapOf(spot.width);
        if (Math.abs(offset.x - spot.x) <= spot.width / 2 + slop && Math.abs(offset.y - spot.y) <= spot.height / 2 + slop) return spot;
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
    const spots = guideSpots(rect, targetId, options, paneRects(model));
    spotsOf.set(options, { key, spots });
    return spots;
}

/** The wide set spots in `zone`, made once for the drops they stand for, like `guideSpotsFor`. */
export function wideSpotsFor(model: LayoutModel, zone: "top" | "bottom", targetId: string, options: DropOption[]): GuideSpot[] {
    const key = `wide-${zone}:${targetId}`;
    const held = spotsOf.get(options);
    if (held?.key === key) return held.spots;
    const spots = wideGuideSpots(zone, targetId, options, paneRects(model));
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
