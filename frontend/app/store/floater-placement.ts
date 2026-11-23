// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where an agent's floating pane goes (docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md §2.1).
 *
 * srv cannot reach a window's screen, so the agent's request arrives in the `openfloatingpane` event and the
 * window resolves it against the work area of its own monitor. Everything here is CSS pixels: the host
 * converts to physical pixels where the platform wants them. A value outside the work area is pulled
 * in, never refused, and the answer says so (`clamped`).
 */
import type { FloaterPlace } from "@/types/rpc/FloaterPlace";

/** The smallest a floating pane is made. */
export const MIN_WIDTH = 320;
export const MIN_HEIGHT = 200;
/** What a floating pane is offset by for each one the agent already has open. */
export const CASCADE_STEP = 32;

export type Rect = { x: number; y: number; width: number; height: number };

export type ResolvedRect = {
    /** Screen coordinates of the window's top-left corner (the same space as the work area's). */
    left: number;
    top: number;
    width: number;
    height: number;
    /** The named place, when the request named one and no x/y overrode it. */
    spot?: string;
    clamped: boolean;
};

type Inputs = {
    /** The work area of the monitor the asking window is on. */
    area: Rect;
    /** What a request with no size gets (about half the asking window). */
    fallback: { width: number; height: number };
    /** The asking window's own rect: where a request with no position is centered. */
    anchor: Rect;
};

/** A size argument in CSS pixels: a number, or a share of `of` (`"60%"`). */
function sizeOf(v: number | string | undefined, of: number): number | undefined {
    if (typeof v === "number") return Number.isFinite(v) && v > 0 ? v : undefined;
    if (typeof v === "string") {
        const m = /^\s*(\d+(?:\.\d+)?)\s*%\s*$/.exec(v);
        if (m) {
            const share = Number(m[1]);
            return share > 0 && share <= 100 ? (of * share) / 100 : undefined;
        }
    }
    return undefined;
}

/** Where a named place puts a window of this size in the work area. */
function spotOrigin(spot: string, area: Rect, width: number, height: number): { left: number; top: number } | undefined {
    const right = area.x + area.width - width;
    const bottom = area.y + area.height - height;
    const midX = area.x + (area.width - width) / 2;
    const midY = area.y + (area.height - height) / 2;
    switch (spot) {
        case "center":
            return { left: midX, top: midY };
        case "right":
            return { left: right, top: midY };
        case "left":
            return { left: area.x, top: midY };
        case "top":
            return { left: midX, top: area.y };
        case "bottom":
            return { left: midX, top: bottom };
        case "top-right":
            return { left: right, top: area.y };
        case "top-left":
            return { left: area.x, top: area.y };
        case "bottom-right":
            return { left: right, top: bottom };
        case "bottom-left":
            return { left: area.x, top: bottom };
        default:
            return undefined;
    }
}

function within(v: number, lo: number, hi: number): number {
    return Math.min(Math.max(v, lo), Math.max(lo, hi));
}

/** Which way a cascade steps from a spot: away from the edge the spot is flush with. */
function cascadeDirection(spot: string | undefined): { dx: number; dy: number } {
    return { dx: spot?.includes("right") ? -1 : 1, dy: spot?.includes("bottom") ? -1 : 1 };
}

/** The window's rect for `place`, in screen CSS pixels, pulled into the work area. */
export function resolveFloaterRect(place: FloaterPlace, inputs: Inputs): ResolvedRect {
    const { area, fallback, anchor } = inputs;
    let clamped = false;

    const wanted = {
        width: sizeOf(place.width, area.width),
        height: sizeOf(place.height, area.height),
    };
    // A size the request did not give is the default, and a default is held to `max_share` when srv set one.
    const cap = place.max_share ?? 1;
    let width = wanted.width ?? fallback.width;
    let height = wanted.height ?? fallback.height;
    if (place.max_share != null) {
        width = Math.min(width, area.width * cap);
        height = Math.min(height, area.height * cap);
    }
    const sized = { width, height };
    width = within(Math.max(width, MIN_WIDTH), MIN_WIDTH, area.width);
    height = within(Math.max(height, MIN_HEIGHT), MIN_HEIGHT, area.height);
    if (width !== sized.width && wanted.width != null) clamped = true;
    if (height !== sized.height && wanted.height != null) clamped = true;

    const exact = place.x != null && place.y != null;
    const spotOrigins = !exact && place.spot ? spotOrigin(place.spot, area, width, height) : undefined;
    let left: number;
    let top: number;
    if (exact) {
        left = area.x + (place.x as number);
        top = area.y + (place.y as number);
    } else if (spotOrigins) {
        ({ left, top } = spotOrigins);
    } else {
        left = anchor.x + Math.max(0, (anchor.width - width) / 2);
        top = anchor.y + Math.max(0, (anchor.height - height) / 2);
    }

    // With no position asked for, each floating pane the agent already has open steps the next one over,
    // and a step that would leave the work area wraps back to the first place.
    if (!exact && place.cascade > 0) {
        const { dx, dy } = cascadeDirection(place.spot);
        const l = left + dx * CASCADE_STEP * place.cascade;
        const t = top + dy * CASCADE_STEP * place.cascade;
        const fits = l >= area.x && t >= area.y && l + width <= area.x + area.width && t + height <= area.y + area.height;
        if (fits) {
            left = l;
            top = t;
        }
    }

    const pulled = {
        left: within(left, area.x, area.x + area.width - width),
        top: within(top, area.y, area.y + area.height - height),
    };
    // A requested position that had to move is a clamp; the default places are not asked for.
    if ((exact || spotOrigins) && (Math.round(pulled.left) !== Math.round(left) || Math.round(pulled.top) !== Math.round(top))) clamped = true;

    return {
        left: Math.round(pulled.left),
        top: Math.round(pulled.top),
        width: Math.round(width),
        height: Math.round(height),
        spot: spotOrigins ? place.spot : undefined,
        clamped,
    };
}
