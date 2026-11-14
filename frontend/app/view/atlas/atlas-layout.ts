// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Atlas's layout (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §5.7): where each region and node stands, in map
 * pixels. It is deterministic and has no physics, so the map does not move when a refresh brings the same agents:
 * regions are placed left to right and wrap to a new row; inside one, the agents that have reports form trees
 * (a lead above its reports, centred over them), and the agents with none and no lead stand in a wrapped grid below.
 * A machine's containers that no agent uses stand at the end of that grid, and the review bot and CI in a region of
 * their own after the machines'.
 */

import type { AtlasEdge, AtlasGraph } from "./atlas-graph";

export const NODE_W = 140;
export const NODE_H = 48;
const GAP_X = 14;
const GAP_Y = 40;
const PAD = 16;
const HEAD = 40;
const FOLD_H = 30;
const REGION_GAP = 28;
/** A row of regions wraps once it would be wider than this. */
export const ROW_MAX_W = 1400;
/** The columns of the grid the agents with no lead and no reports stand in. */
const GRID_COLS = 5;
const MIN_REGION_W = 220;
/** The key of the region the review bot and CI stand in. */
export const SERVICES_REGION = "services";

export interface Placed {
    x: number;
    y: number;
}

export interface RegionBox extends Placed {
    key: string;
    w: number;
    h: number;
    /** Where the folded chip sits (its top left); unused when the region folds nothing. */
    foldY: number;
}

export interface AtlasLayout {
    regions: RegionBox[];
    /** Every node's top left, by agent key. */
    nodes: Map<string, Placed>;
    width: number;
    height: number;
}

/** The trees of one region: each root that has reports, with the subtree it heads, in the order given. */
function treesOf(keys: readonly string[], edges: readonly AtlasEdge[]) {
    const inRegion = new Set(keys);
    const children = new Map<string, string[]>();
    const hasParent = new Set<string>();
    for (const e of edges) {
        if (!inRegion.has(e.from) || !inRegion.has(e.to) || e.from === e.to || hasParent.has(e.to)) continue;
        children.set(e.from, [...(children.get(e.from) ?? []), e.to]);
        hasParent.add(e.to);
    }
    const roots = keys.filter((k) => !hasParent.has(k) && children.has(k));
    const loners = keys.filter((k) => !hasParent.has(k) && !children.has(k));
    // srv refuses a cycle, but a seat changing between two reads can show one for a moment: what no root reaches
    // stands with the loners, not lost.
    const reached = new Set<string>();
    const walk = (k: string) => {
        if (reached.has(k)) return;
        reached.add(k);
        for (const c of children.get(k) ?? []) walk(c);
    };
    roots.forEach(walk);
    for (const k of loners) reached.add(k);
    const stray = keys.filter((k) => !reached.has(k));
    return { roots, loners: [...loners, ...stray], children };
}

interface RegionLayout {
    nodes: Map<string, Placed>;
    w: number;
    h: number;
}

/** One region's nodes at positions relative to its top left, and the size its nodes need. */
function layoutRegion(keys: readonly string[], edges: readonly AtlasEdge[]): RegionLayout {
    const { roots, loners, children } = treesOf(keys, edges);
    const nodes = new Map<string, Placed>();
    let slot = 0;
    let depthMax = -1;
    const place = (k: string, depth: number, seen: Set<string>): number => {
        seen.add(k);
        depthMax = Math.max(depthMax, depth);
        const kids = (children.get(k) ?? []).filter((c) => !seen.has(c));
        let x: number;
        if (kids.length === 0) {
            x = slot++ * (NODE_W + GAP_X);
        } else {
            const xs = kids.map((c) => place(c, depth + 1, seen));
            x = (xs[0] + xs[xs.length - 1]) / 2;
        }
        nodes.set(k, { x, y: depth * (NODE_H + GAP_Y) });
        return x;
    };
    const seen = new Set<string>();
    for (const r of roots) place(r, 0, seen);
    const treeW = slot === 0 ? 0 : slot * (NODE_W + GAP_X) - GAP_X;
    const treeH = depthMax < 0 ? 0 : (depthMax + 1) * (NODE_H + GAP_Y) - GAP_Y;

    // The agents with no lead and no reports, in a grid under the trees.
    const cols = Math.min(GRID_COLS, Math.max(loners.length, 1));
    const gridTop = treeH > 0 ? treeH + GAP_Y : 0;
    loners.forEach((k, i) => {
        nodes.set(k, { x: (i % cols) * (NODE_W + GAP_X), y: gridTop + Math.floor(i / cols) * (NODE_H + GAP_X) });
    });
    const gridW = loners.length === 0 ? 0 : cols * (NODE_W + GAP_X) - GAP_X;
    const gridH = loners.length === 0 ? 0 : Math.ceil(loners.length / cols) * (NODE_H + GAP_X) - GAP_X;
    return { nodes, w: Math.max(treeW, gridW), h: gridTop + gridH };
}

export function layoutAtlas(graph: AtlasGraph): AtlasLayout {
    const boxes: RegionBox[] = [];
    const nodes = new Map<string, Placed>();
    let x = 0;
    let y = 0;
    let rowH = 0;
    let width = 0;
    const place = (key: string, keys: readonly string[], folds: boolean) => {
        const inner = layoutRegion(keys, graph.edges);
        const w = Math.max(MIN_REGION_W, inner.w + PAD * 2);
        const innerH = keys.length === 0 ? 0 : inner.h;
        const h = HEAD + PAD + innerH + (folds ? (innerH > 0 ? GAP_X : 0) + FOLD_H : 0) + PAD;
        if (x > 0 && x + w > ROW_MAX_W) {
            x = 0;
            y += rowH + REGION_GAP;
            rowH = 0;
        }
        // The nodes are centred in the region when it is wider than they are.
        const offset = PAD + (w - PAD * 2 - inner.w) / 2;
        for (const [node, p] of inner.nodes) nodes.set(node, { x: x + offset + p.x, y: y + HEAD + PAD + p.y });
        boxes.push({ key, x, y, w, h, foldY: y + HEAD + PAD + innerH + (innerH > 0 ? GAP_X : 0) });
        x += w + REGION_GAP;
        rowH = Math.max(rowH, h);
        width = Math.max(width, x - REGION_GAP);
    };
    for (const region of graph.regions) {
        const keys = [...region.shown, ...region.orphans].map((n) => n.key);
        place(region.key, keys, region.folded.length > 0);
    }
    const services = graph.services.map((s) => s.key);
    if (services.length > 0) place(SERVICES_REGION, services, false);
    return { regions: boxes, nodes, width, height: boxes.length === 0 ? 0 : y + rowH };
}

/**
 * The By seat layout: every shown agent in one picture, whatever machine it is on, a lead above its reports. No
 * regions: the machine is the node's tint. Stopped agents are not drawn (the bar counts them). The containers no
 * agent uses, and the review bot and CI, stand with the agents that have no lead.
 */
export function layoutBySeat(graph: AtlasGraph): AtlasLayout {
    const keys = [
        ...graph.regions.flatMap((r) => r.shown.map((a) => a.key)),
        ...graph.regions.flatMap((r) => r.orphans.map((o) => o.key)),
        ...graph.services.map((s) => s.key),
    ];
    const inner = layoutRegion(keys, graph.edges);
    const nodes = new Map<string, Placed>();
    for (const [key, p] of inner.nodes) nodes.set(key, { x: PAD + p.x, y: PAD + p.y });
    return { regions: [], nodes, width: keys.length === 0 ? 0 : inner.w + PAD * 2, height: keys.length === 0 ? 0 : inner.h + PAD * 2 };
}

/** Where the folded chip stands in its region's box: its left and its size. */
export const FOLD_CHIP = { x: PAD, h: FOLD_H - 6 };
