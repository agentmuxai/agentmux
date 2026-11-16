// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Atlas's layout (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §5.7): where each region and node stands, in map
 * pixels. It is deterministic and has no physics, so the map does not move when a refresh brings the same agents:
 * regions are placed left to right and wrap to a new row; inside one, the agents that have reports form trees
 * (a lead above its reports, centred over them), and the agents with none and no lead stand in a wrapped grid below.
 * A machine's containers that no agent uses stand at the end of that grid. The review bot and CI stand in a region
 * of their own on a row of its own under the machines', so that a line from one of them to an agent crosses open
 * map and does not run along the row the agent stands in, behind its neighbours.
 */

import type { AtlasEdge, AtlasGraph } from "./atlas-graph";

export const NODE_W = 140;
/** A name and a line under it, with a few pixels over and under them: the box is no taller than its words need. */
export const NODE_H = 36;
/** How far the dashed frame of a container agent reaches past its node (atlas.scss `.atlas-node--container`). */
export const CONTAINER_FRAME_OUT = 7;
/** The gap between two nodes of a row, and between one level and the next. */
export const GAP_X = 14;
export const GAP_Y = 40;
/** Between the rows a lead's reports are wrapped into. */
const REPORT_ROW_GAP = 24;
/** Between a region's edge and its nodes: room for a container's frame (CONTAINER_FRAME_OUT) and little more. */
const PAD = 10;
/** The strip at the top of a region that its title and badge are written in. */
export const REGION_TITLE_H = 30;
const HEAD = REGION_TITLE_H;
const FOLD_H = 30;
const REGION_GAP = 28;
/** Between the agents and the row of the review bot and CI in the By seat layout: room for the labels of their lines. */
const SERVICES_GAP = 72;
/** A row of regions wraps once it would be wider than this. */
export const ROW_MAX_W = 1400;
/** The columns of the grid the agents with no lead and no reports stand in. */
const GRID_COLS = 5;
const MIN_REGION_W = 220;
/** The key of the region the review bot and CI stand in. */
export const SERVICES_REGION = "services";

/**
 * The gaps of the map. They are the ones above in the fitted view, and every one closes by the same share as the
 * person zooms in past it (`gapsAt`), so that a gap takes no more of the pane as the cards grow and more of them
 * are in view. The cards, a region's title strip and its folded chip keep their size.
 */
export interface Gaps {
    /** The share of each full gap that is left: 1 in the fitted view. */
    spread: number;
    /** Between two nodes of a row, and between two rows of the grid of agents with no lead. */
    x: number;
    /** Between a lead's row and its reports'. */
    level: number;
    /** Between the rows a lead's reports are wrapped into. */
    reportRow: number;
    /** Between a region's edge and its nodes. */
    pad: number;
    /** Between two regions, side by side or one row under another. */
    region: number;
    /** Between the agents and the review bot and CI in the By seat layout. */
    services: number;
    /** How far the dashed frame of a container agent reaches past its node. */
    frame: number;
}

const FULL_GAPS: Gaps = {
    spread: 1,
    x: GAP_X,
    level: GAP_Y,
    reportRow: REPORT_ROW_GAP,
    pad: PAD,
    region: REGION_GAP,
    services: SERVICES_GAP,
    frame: CONTAINER_FRAME_OUT,
};
/** The least share of a gap that is left, however far the person zooms in: a line and a frame still fit in it. */
export const MIN_SPREAD = 0.4;

/** The gaps at `spread`: the full ones at 1 or more, and under 1 that share of each, no less than `MIN_SPREAD`. */
export function gapsAt(spread = 1): Gaps {
    if (!(spread < 1)) return FULL_GAPS;
    const s = Math.max(MIN_SPREAD, spread);
    return {
        spread: s,
        x: GAP_X * s,
        level: GAP_Y * s,
        reportRow: REPORT_ROW_GAP * s,
        pad: PAD * s,
        region: REGION_GAP * s,
        services: SERVICES_GAP * s,
        frame: CONTAINER_FRAME_OUT * s,
    };
}

export interface Placed {
    x: number;
    y: number;
}

export interface RegionBox extends Placed {
    key: string;
    w: number;
    h: number;
    /** Where the folded chip sits (its top left); unused when the region folds nothing. */
    foldX: number;
    foldY: number;
}

export interface AtlasLayout {
    regions: RegionBox[];
    /** Every node's top left, by agent key. */
    nodes: Map<string, Placed>;
    width: number;
    height: number;
    /**
     * The reports the layout wrapped onto a later row than the one under their lead, by report key, with the lead's key:
     * only the line from that lead to that report is drawn down the gap beside the rows above it (atlas-edge-path.ts).
     */
    wrapped: ReadonlyMap<string, string>;
    /** The gaps it was made with: a wrapped report's line runs along the middle of two of them. */
    gaps: Gaps;
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
    /** The reports wrapped onto a later row than the one under their lead, by report key, with their lead's key. */
    wrapped: Map<string, string>;
}

/**
 * One region's nodes at positions relative to its top left, and the size its nodes need. `reportCols` is the most
 * reports a lead keeps in one row: a lead whose reports are all agents with none of their own, and more than that many,
 * stands them in rows of `reportCols` under it (centred), so a machine with a wide lead need not be as wide as its
 * reports side by side. Unlimited, a lead's reports are one row.
 */
function layoutRegion(
    keys: readonly string[],
    edges: readonly AtlasEdge[],
    gridCols = GRID_COLS,
    reportCols = Number.POSITIVE_INFINITY,
    gaps: Gaps = FULL_GAPS
): RegionLayout {
    const { roots, loners, children } = treesOf(keys, edges);
    const nodes = new Map<string, Placed>();
    const wrapped = new Map<string, string>();
    let slot = 0;
    /** The lowest edge of a node the trees have placed. */
    let bottom = 0;
    const put = (k: string, x: number, y: number) => {
        nodes.set(k, { x, y });
        bottom = Math.max(bottom, y + NODE_H);
    };
    const place = (k: string, depth: number, seen: Set<string>): number => {
        seen.add(k);
        const kids = (children.get(k) ?? []).filter((c) => !seen.has(c));
        let x: number;
        if (kids.length === 0) {
            x = slot++ * (NODE_W + gaps.x);
        } else if (kids.length > reportCols && kids.every((c) => (children.get(c) ?? []).length === 0)) {
            const first = slot;
            kids.forEach((c, i) => {
                seen.add(c);
                const row = Math.floor(i / reportCols);
                if (row > 0) wrapped.set(c, k);
                put(c, (first + (i % reportCols)) * (NODE_W + gaps.x), (depth + 1) * (NODE_H + gaps.level) + row * (NODE_H + gaps.reportRow));
            });
            slot += reportCols;
            x = ((first + (first + reportCols - 1)) / 2) * (NODE_W + gaps.x);
        } else {
            const xs = kids.map((c) => place(c, depth + 1, seen));
            x = (xs[0] + xs[xs.length - 1]) / 2;
        }
        put(k, x, depth * (NODE_H + gaps.level));
        return x;
    };
    const seen = new Set<string>();
    for (const r of roots) place(r, 0, seen);
    const treeW = slot === 0 ? 0 : slot * (NODE_W + gaps.x) - gaps.x;
    const treeH = bottom;

    // The agents with no lead and no reports, in a grid under the trees.
    const cols = Math.min(gridCols, Math.max(loners.length, 1));
    const gridTop = treeH > 0 ? treeH + gaps.level : 0;
    loners.forEach((k, i) => {
        nodes.set(k, { x: (i % cols) * (NODE_W + gaps.x), y: gridTop + Math.floor(i / cols) * (NODE_H + gaps.x) });
    });
    const gridW = loners.length === 0 ? 0 : cols * (NODE_W + gaps.x) - gaps.x;
    const gridH = loners.length === 0 ? 0 : Math.ceil(loners.length / cols) * (NODE_H + gaps.x) - gaps.x;
    return { nodes, w: Math.max(treeW, gridW), h: gridTop + gridH, wrapped };
}

/** A pane's size, as the layout is formed for it. */
export interface PaneShape {
    w: number;
    h: number;
}

/**
 * The smallest scale the fitted view shrinks the map to: a card is then 84 by 22 px, the smallest where a name and
 * a state still read. Under it the view fills the pane in the shorter direction and the person pans along the other.
 */
export const MIN_READABLE_K = 0.6;
/** The largest scale a fit enlarges a small map to. */
export const FIT_MAX_K = 1.5;
/** The margin the fitted view leaves round the map (atlas-viewport.ts, FIT_MARGIN). */
const FIT_PAD = 16;
/** The columns the grid of agents with no lead may have, tried in turn. */
const GRID_COLS_CHOICES = [2, 3, 4, 5, 6];
/** The same for the By seat picture, which has no machine boxes to keep narrow: a wide pane may take far more columns, so its cards can grow (#47793). */
const SEAT_GRID_COLS_CHOICES = [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 16];
/** The most reports of a lead one row may hold, tried in turn after the unwrapped row. */
const REPORT_COLS_CHOICES = [6, 4, 3, 2];

/**
 * The widths a lead's reports may be wrapped at: unlimited first, then each of REPORT_COLS_CHOICES that some lead has
 * more reports than (a wrap that changes nothing is not a candidate).
 */
function reportColsChoices(graph: AtlasGraph): number[] {
    const reports = new Map<string, number>();
    for (const e of graph.edges) reports.set(e.from, (reports.get(e.from) ?? 0) + 1);
    const widest = Math.max(0, ...reports.values());
    return [Number.POSITIVE_INFINITY, ...REPORT_COLS_CHOICES.filter((c) => widest > c)];
}

/** A region's box before it is placed: its inner layout and the size it needs. */
interface Slot {
    key: string;
    folds: boolean;
    inner: RegionLayout;
    w: number;
    h: number;
    innerH: number;
}

function slotOf(
    key: string,
    keys: readonly string[],
    folds: boolean,
    edges: readonly AtlasEdge[],
    cols: number,
    reportCols = Number.POSITIVE_INFINITY,
    gaps: Gaps = FULL_GAPS
): Slot {
    const inner = layoutRegion(keys, edges, cols, reportCols, gaps);
    const w = Math.max(MIN_REGION_W, inner.w + gaps.pad * 2);
    const innerH = keys.length === 0 ? 0 : inner.h;
    const h = HEAD + gaps.pad + innerH + (folds ? (innerH > 0 ? gaps.x : 0) + FOLD_H : 0) + gaps.pad;
    return { key, folds, inner, w, h, innerH };
}

/** The slots a packed layout started a new row of regions at, by their index: what the same picture at closer gaps keeps. */
const ROW_STARTS = new WeakMap<AtlasLayout, ReadonlySet<number>>();

/**
 * The slots in rows; the review bot and CI (`services`) stand in a row of their own after the machines. `wrap` is a
 * width (a row wraps once it would be wider) or the slots that start a new row: a picture rebuilt at closer gaps is
 * narrower, and keeps its rows by where they broke, not by a width.
 */
function shelfPack(
    slots: readonly Slot[],
    services: Slot | null,
    wrap: number | ReadonlySet<number>,
    gaps: Gaps = FULL_GAPS
): AtlasLayout {
    const boxes: RegionBox[] = [];
    const nodes = new Map<string, Placed>();
    const wrapped = new Map<string, string>();
    const rowStarts = new Set<number>();
    let x = 0;
    let y = 0;
    let rowH = 0;
    let width = 0;
    const newRow = () => {
        x = 0;
        y += rowH + gaps.region;
        rowH = 0;
    };
    const place = (s: Slot, i: number) => {
        if (typeof wrap === "number" ? x > 0 && x + s.w > wrap : wrap.has(i)) {
            newRow();
            rowStarts.add(i);
        }
        // The nodes are centred in the region when it is wider than they are.
        const offset = gaps.pad + (s.w - gaps.pad * 2 - s.inner.w) / 2;
        const top = y + HEAD + gaps.pad;
        for (const [node, p] of s.inner.nodes) nodes.set(node, { x: x + offset + p.x, y: top + p.y });
        for (const [report, lead] of s.inner.wrapped) wrapped.set(report, lead);
        const foldY = top + s.innerH + (s.innerH > 0 ? gaps.x : 0);
        boxes.push({ key: s.key, x, y, w: s.w, h: s.h, foldX: x + gaps.pad, foldY });
        x += s.w + gaps.region;
        rowH = Math.max(rowH, s.h);
        width = Math.max(width, x - gaps.region);
    };
    slots.forEach(place);
    if (services) {
        if (x > 0) newRow();
        place(services, slots.length);
    }
    const layout = { regions: boxes, nodes, width, height: boxes.length === 0 ? 0 : y + rowH, wrapped, gaps };
    ROW_STARTS.set(layout, rowStarts);
    return layout;
}

/** The scale the fitted view would show `layout` at in `pane`, before the readable floor. */
export function fitScale(layout: { width: number; height: number }, pane: PaneShape): number {
    if (layout.width <= 0 || layout.height <= 0) return FIT_MAX_K;
    return Math.min((pane.w - FIT_PAD * 2) / layout.width, (pane.h - FIT_PAD * 2) / layout.height);
}

/** Two pictures whose cards fill their box within this share of each other count as equally full. */
const DENSE_EPS = 0.01;

const rowsOf = (l: AtlasLayout): number => new Set(l.regions.map((r) => r.y)).size;

/**
 * The candidate that fits `pane` best: the largest scale (no larger than a fit enlarges to, and no smaller than
 * a card reads at, where the overflow is then what counts), then the least empty room inside the picture (cards over
 * its bounding box: a taller, narrower grid under wide trees has the same scale and covers more of the pane, but half
 * of its box is empty), then the most of the pane covered, then the fewest rows.
 */
export function pickBest(candidates: readonly AtlasLayout[], pane: PaneShape, preferFull = false): AtlasLayout {
    const score = (l: AtlasLayout) => {
        const k = Math.max(MIN_READABLE_K, Math.min(FIT_MAX_K, fitScale(l, pane)));
        const raw = fitScale(l, pane);
        // Only the By seat picture (no region boxes) asks for it; the machine boxes have room of their own.
        const dense =
            preferFull && l.width * l.height > 0 ? (l.nodes.size * NODE_W * NODE_H) / (l.width * l.height) : 0;
        return { k, raw, dense, cover: Math.min(l.width * k, pane.w) * Math.min(l.height * k, pane.h), rows: rowsOf(l) };
    };
    let best = candidates[0];
    let bs = score(best);
    for (const c of candidates.slice(1)) {
        const s = score(c);
        // Under the floor every candidate shows at the floor, and the one that overflows least is the best.
        const better =
            s.k > bs.k + 1e-6 ||
            (Math.abs(s.k - bs.k) <= 1e-6 && s.k <= MIN_READABLE_K + 1e-6 && s.raw > bs.raw + 1e-6) ||
            (Math.abs(s.k - bs.k) <= 1e-6 &&
                Math.abs(s.raw - bs.raw) <= 1e-6 &&
                (s.dense > bs.dense + DENSE_EPS ||
                    (Math.abs(s.dense - bs.dense) <= DENSE_EPS &&
                        (s.cover > bs.cover + 1 || (Math.abs(s.cover - bs.cover) <= 1 && s.rows < bs.rows)))));
        if (better) {
            best = c;
            bs = s;
        }
    }
    return best;
}

/** The widths a row of the slots may wrap at: the one that holds the first box, the first two, and so on. */
function wrapWidths(slots: readonly Slot[]): number[] {
    const out = new Set<number>();
    let sum = 0;
    for (const s of slots) {
        sum += s.w;
        out.add(sum);
        sum += REGION_GAP;
    }
    return [...out];
}

/**
 * Where each region and node stands. With the pane's size the layout is formed for it: the machine boxes keep their
 * order and their insides, and the width a row wraps at and the columns of the grid of agents with no lead are the
 * ones that fit the pane best (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §6.5). Without it, the fixed ones.
 * `spread` under 1 closes the gaps (`gapsAt`): the picture is the one chosen at the full gaps, so a zoom moves no
 * card to another row or column and no machine to another row.
 */
export function layoutAtlas(graph: AtlasGraph, pane?: PaneShape, spread = 1): AtlasLayout {
    const gaps = gapsAt(spread);
    const build = (cols: number, reportCols: number, g: Gaps) => {
        const slots = graph.regions.map((r) =>
            slotOf(
                r.key,
                [...r.shown, ...r.orphans].map((n) => n.key),
                r.folded.length > 0,
                graph.edges,
                cols,
                reportCols,
                g
            )
        );
        const svc = graph.services.map((s) => s.key);
        return {
            slots,
            services: svc.length > 0 ? slotOf(SERVICES_REGION, svc, false, graph.edges, cols, reportCols, g) : null,
        };
    };
    const packed = (cols: number, reportCols: number, wrap: number | ReadonlySet<number>, g: Gaps): AtlasLayout => {
        const { slots, services } = build(cols, reportCols, g);
        return shelfPack(slots, services, wrap, g);
    };
    if (!pane || pane.w <= 0 || pane.h <= 0) return packed(GRID_COLS, Number.POSITIVE_INFINITY, ROW_MAX_W, gaps);
    const candidates: AtlasLayout[] = [];
    const closed: (() => AtlasLayout)[] = [];
    // Unwrapped first: with nothing to gain from wrapping a lead's reports, the pick keeps the look it had.
    for (const reportCols of reportColsChoices(graph)) {
        for (const cols of GRID_COLS_CHOICES) {
            const { slots, services } = build(cols, reportCols, FULL_GAPS);
            // With no machine box (an empty or not-yet-loaded fleet, or only the services) there is no width to choose.
            const wraps = slots.length > 0 ? wrapWidths(slots) : [ROW_MAX_W];
            for (const wrapW of wraps) {
                const full = shelfPack(slots, services, wrapW);
                candidates.push(full);
                closed.push(() => packed(cols, reportCols, ROW_STARTS.get(full) ?? wrapW, gaps));
            }
        }
    }
    const best = pickBest(candidates, pane);
    return gaps === FULL_GAPS ? best : closed[candidates.indexOf(best)]();
}

/**
 * The By seat layout: every shown agent in one picture, whatever machine it is on, a lead above its reports. No
 * regions: the machine is the node's tint. Stopped agents are not drawn (the bar counts them). The containers no
 * agent uses stand with the agents that have no lead, and the review bot and CI in a row of their own under them
 * all, for the same reason as their region in the other layout. With the pane's size, the columns of the grid of
 * agents with no lead are the ones that fit it best. `spread` under 1 closes the gaps, as in `layoutAtlas`: the
 * columns are the ones chosen at the full gaps.
 */
export function layoutBySeat(graph: AtlasGraph, pane?: PaneShape, spread = 1): AtlasLayout {
    const gaps = gapsAt(spread);
    if (!pane || pane.w <= 0 || pane.h <= 0) return seatWithCols(graph, GRID_COLS, Number.POSITIVE_INFINITY, gaps);
    const choices = reportColsChoices(graph).flatMap((reportCols) =>
        SEAT_GRID_COLS_CHOICES.map((cols) => ({ cols, reportCols }))
    );
    const candidates = choices.map((c) => seatWithCols(graph, c.cols, c.reportCols));
    const best = pickBest(candidates, pane, true);
    if (gaps === FULL_GAPS) return best;
    const chosen = choices[candidates.indexOf(best)];
    return seatWithCols(graph, chosen.cols, chosen.reportCols, gaps);
}

function seatWithCols(
    graph: AtlasGraph,
    cols: number,
    reportCols = Number.POSITIVE_INFINITY,
    gaps: Gaps = FULL_GAPS
): AtlasLayout {
    const keys = [
        ...graph.regions.flatMap((r) => r.shown.map((a) => a.key)),
        ...graph.regions.flatMap((r) => r.orphans.map((o) => o.key)),
    ];
    const services = graph.services.map((s) => s.key);
    const inner = layoutRegion(keys, graph.edges, cols, reportCols, gaps);
    const nodes = new Map<string, Placed>();
    for (const [key, p] of inner.nodes) nodes.set(key, { x: gaps.pad + p.x, y: gaps.pad + p.y });
    const servicesTop = keys.length === 0 ? 0 : inner.h + gaps.services;
    services.forEach((key, i) => nodes.set(key, { x: gaps.pad + i * (NODE_W + gaps.x), y: gaps.pad + servicesTop }));
    if (nodes.size === 0) return { regions: [], nodes, width: 0, height: 0, wrapped: new Map(), gaps };
    const servicesW = services.length === 0 ? 0 : services.length * (NODE_W + gaps.x) - gaps.x;
    const h = services.length === 0 ? inner.h : servicesTop + NODE_H;
    return {
        regions: [],
        nodes,
        width: Math.max(inner.w, servicesW) + gaps.pad * 2,
        height: h + gaps.pad * 2,
        wrapped: inner.wrapped,
        gaps,
    };
}

/** The folded chip's height in its region's box (`RegionBox.foldX` and `foldY` say where it stands). */
export const FOLD_CHIP = { h: FOLD_H - 6 };
