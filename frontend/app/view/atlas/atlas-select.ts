// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Selecting agents on the map (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §5.6): a box dragged over the nodes
 * adds the agents it touches to the Swarm's own selection, which Broadcast, Stop and the Supervisor's mode act on.
 * This file is pure: the box in pane pixels, the same box on the map, and the nodes it touches.
 */

import type { AtlasAgent } from "./atlas-graph";
import { NODE_H, NODE_W, type Placed } from "./atlas-layout";
import type { Viewport } from "./atlas-viewport";

export interface Rect {
    x: number;
    y: number;
    w: number;
    h: number;
}

/** A drag shorter than this, both ways, is a press and selects nothing. */
export const LASSO_MIN_PX = 4;

/** The box between a drag's start and where the pointer is now, whichever way it was dragged. */
export function rectBetween(ax: number, ay: number, bx: number, by: number): Rect {
    return { x: Math.min(ax, bx), y: Math.min(ay, by), w: Math.abs(bx - ax), h: Math.abs(by - ay) };
}

export function isDrag(rect: Rect): boolean {
    return rect.w >= LASSO_MIN_PX || rect.h >= LASSO_MIN_PX;
}

/** A box of the pane, in the map's own pixels under `view`. */
export function toMapRect(rect: Rect, view: Viewport): Rect {
    return { x: (rect.x - view.x) / view.k, y: (rect.y - view.y) / view.k, w: rect.w / view.k, h: rect.h / view.k };
}

/** The keys of the nodes a box of the map touches, in the order the layout holds them. */
export function nodesInRect(nodes: ReadonlyMap<string, Placed>, rect: Rect): string[] {
    const out: string[] = [];
    for (const [key, p] of nodes) {
        const apart = p.x > rect.x + rect.w || p.x + NODE_W < rect.x || p.y > rect.y + rect.h || p.y + NODE_H < rect.y;
        if (!apart) out.push(key);
    }
    return out;
}

/**
 * The key the Swarm's selection knows an agent by, or null for one it cannot select: this instance's agent is its
 * pane's block id (one with no pane has no checkbox in the Swarm either), another instance's is its row key, and
 * one whose machine has gone stale is about to leave the list.
 */
export function selectionKeyOf(agent: AtlasAgent): string | null {
    if (agent.state === "stopped") return null;
    if (agent.remote) return agent.stale ? null : agent.key;
    return agent.blockId;
}
