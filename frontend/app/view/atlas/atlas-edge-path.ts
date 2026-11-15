// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The line from a lead's node to a report's. A report on the row under its lead is reached by a curve from the lead's
 * bottom to the report's top. A report the layout wrapped onto a later row (`reportCols` in atlas-layout.ts) would
 * have that curve run behind the cards of the rows above it and come out of the bottom of one of them, as if it were
 * that card's report; its line instead runs along the top of the first row, down the gap left of the report's column,
 * and into the card's left side, so it crosses no card. Which reports were wrapped is the layout's to say
 * (`AtlasLayout.wrapped`): a line between agents of two machines, or to a report of another lead, can also run far
 * down the map, and is still the curve.
 */

import { GAP_X, GAP_Y, NODE_H, NODE_W } from "./atlas-layout";

export interface Pt {
    x: number;
    y: number;
}

/** How round the corners of a wrapped report's line are. */
const CORNER_R = 8;

/**
 * The corners of the line to a report the layout wrapped onto a later row. The line leaves the lead's bottom, runs
 * along the middle of the gap above the first row, goes down the gap left of the report's column to the middle of its
 * card, and ends at the card's left edge.
 */
export function wrappedRoute(from: Pt, to: Pt): Pt[] {
    const x1 = from.x + NODE_W / 2;
    const y1 = from.y + NODE_H;
    const bus = y1 + GAP_Y / 2;
    const railX = to.x - GAP_X / 2;
    const yMid = to.y + NODE_H / 2;
    const pts: Pt[] = [{ x: x1, y: y1 }];
    if (Math.abs(railX - x1) >= 1) pts.push({ x: x1, y: bus }, { x: railX, y: bus });
    pts.push({ x: railX, y: yMid }, { x: to.x, y: yMid });
    return pts;
}

/** A polyline as a path with rounded corners. */
function roundedPath(pts: readonly Pt[]): string {
    let d = `M${pts[0].x},${pts[0].y}`;
    for (let i = 1; i < pts.length; i++) {
        const prev = pts[i - 1];
        const at = pts[i];
        const next = pts[i + 1];
        if (!next) {
            d += ` L${at.x},${at.y}`;
            continue;
        }
        const inLen = Math.hypot(at.x - prev.x, at.y - prev.y);
        const outLen = Math.hypot(next.x - at.x, next.y - at.y);
        const r = Math.min(CORNER_R, inLen / 2, outLen / 2);
        const before = { x: at.x - ((at.x - prev.x) / inLen) * r, y: at.y - ((at.y - prev.y) / inLen) * r };
        const after = { x: at.x + ((next.x - at.x) / outLen) * r, y: at.y + ((next.y - at.y) / outLen) * r };
        d += ` L${before.x},${before.y} Q${at.x},${at.y} ${after.x},${after.y}`;
    }
    return d;
}

/** The path of a line from a lead's node to a report's; `wrapped`: the layout wrapped that report onto a later row. */
export function edgePath(from: Pt, to: Pt, wrapped = false): string {
    if (wrapped) return roundedPath(wrappedRoute(from, to));
    const x1 = from.x + NODE_W / 2;
    const y1 = from.y + NODE_H;
    const x2 = to.x + NODE_W / 2;
    const y2 = to.y;
    const mid = (y1 + y2) / 2;
    return `M${x1},${y1} C${x1},${mid} ${x2},${mid} ${x2},${y2}`;
}
