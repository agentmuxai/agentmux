// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The dragged pane as it stands on screen in its landing rect, once the live reflow has brought every tile to
 * the drop being shown (liveReflow.ts `landingSettled`). The pane in hand is a picture, and a picture of the
 * pane taken where it was picked up is the wrong shape for most landing rects; this says where a picture of
 * it laid out for the rect can be taken (app/drag/pane-live-follow.ts).
 *
 * The landing zone is drawn over the pane there (tilelayout.scss `.placeholder`): a border, and a fill of one
 * colour at one strength. Both are read from the zone as it is drawn, not assumed: the picture is taken
 * inside the border, and the fill is handed over as the `wash` to take back out of it. A zone that is still
 * arriving, or drawn in a way that cannot be taken out (not a plain colour), gives no site, and the pane in
 * hand keeps the picture it has.
 */

import type { LayoutModel } from "./layoutModel";

export interface SiteRect {
    left: number;
    top: number;
    width: number;
    height: number;
}

/** A flat colour drawn over the pane at strength `a` (0 to 1). */
export interface Wash {
    r: number;
    g: number;
    b: number;
    a: number;
}

export interface LandingSite {
    /** The pane on screen, in the viewport's CSS px, inside the landing zone's border. */
    rect: SiteRect;
    /** The viewport's width, CSS px: what a picture of the window is scaled by to find `rect` in it. */
    viewportWidth: number;
    /** The landing zone's fill over it; none when nothing is drawn over the pane. */
    wash?: Wash;
}

/** A computed `rgb()` or `rgba()` colour; none for any other form. */
export function parseWash(color: string): Wash | undefined {
    const m = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(color.trim());
    if (!m) return undefined;
    const [r, g, b] = [Number(m[1]), Number(m[2]), Number(m[3])];
    const a = m[4] === undefined ? 1 : Number(m[4]);
    return [r, g, b, a].every(Number.isFinite) ? { r, g, b, a } : undefined;
}

/** Whether a computed transform leaves the element where and as large as it is laid out. */
function atRest(transform: string): boolean {
    return transform === "none" || transform === "" || transform === "matrix(1, 0, 0, 1, 0, 0)";
}

function intersect(a: SiteRect, b: SiteRect): SiteRect | undefined {
    const left = Math.max(a.left, b.left);
    const top = Math.max(a.top, b.top);
    const right = Math.min(a.left + a.width, b.left + b.width);
    const bottom = Math.min(a.top + a.height, b.top + b.height);
    return right > left && bottom > top ? { left, top, width: right - left, height: bottom - top } : undefined;
}

/** The least of the pane a picture must show to be the pane: less, and the zone is not the one over it. */
const MIN_SHARE = 0.9;

/**
 * Where `pane` is on screen under `zone` (the landing zone's inner element, or none when none is drawn), and
 * what the zone draws over it. None when the zone is not at rest over the pane.
 */
export function siteUnderZone(pane: Element, zone: Element | null): LandingSite | undefined {
    const p = pane.getBoundingClientRect();
    const paneRect = { left: p.left, top: p.top, width: p.width, height: p.height };
    if (!(paneRect.width > 0) || !(paneRect.height > 0)) return undefined;
    const viewportWidth = window.innerWidth;
    if (!zone) return { rect: paneRect, viewportWidth };
    const style = getComputedStyle(zone);
    if (Number(style.opacity) !== 1 || !atRest(style.transform)) return undefined;
    const wash = parseWash(style.backgroundColor);
    if (!wash || wash.a >= 1) return undefined;
    const z = zone.getBoundingClientRect();
    const px = (v: string) => Number.parseFloat(v) || 0;
    const [bl, bt, br, bb] = [px(style.borderLeftWidth), px(style.borderTopWidth), px(style.borderRightWidth), px(style.borderBottomWidth)];
    // A px more than the border on each side: the zone and the pane are rounded to whole px each on its own.
    const inner = { left: z.left + bl + 1, top: z.top + bt + 1, width: z.width - bl - br - 2, height: z.height - bt - bb - 2 };
    const rect = intersect(paneRect, inner);
    if (!rect || rect.width * rect.height < paneRect.width * paneRect.height * MIN_SHARE) return undefined;
    return wash.a > 0 ? { rect, viewportWidth, wash } : { rect, viewportWidth };
}

/**
 * The pane of `blockId` in its landing rect in `model`'s layout, read from the screen now. None until the
 * panes stand in the drop being shown, and none when the pane or the zone cannot be read.
 */
export function landingSiteOf(model: Pick<LayoutModel, "landingSettled" | "displayContainerRef">, blockId: string): LandingSite | undefined {
    if (!model.landingSettled()) return undefined;
    const layout = model.displayContainerRef?.current?.parentElement;
    // By the attribute's value, not a selector built from it: nothing to escape.
    const pane = Array.from(layout?.querySelectorAll("[data-blockid]") ?? []).find((el) => el.getAttribute("data-blockid") === blockId);
    if (!layout || !pane) return undefined;
    return siteUnderZone(pane, layout.querySelector(".placeholder-container .placeholder"));
}
