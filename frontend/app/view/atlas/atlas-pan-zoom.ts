// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The map's pan and zoom state for the Atlas pane. The map fits the pane, and follows the pane and the agents,
 * until the person pans or zooms it: from then on it stays where they put it, and Fit (or the layout toggle) takes
 * it back. The wheel zooms about the pointer, the zoom buttons zoom with the map centred in the pane, and a drag
 * on the background pans. Zoomed in past the fitted view, the map's gaps close up (`spread`), so the
 * layout is a new one at each scale: a zoom keeps the node under the pointer, or the map's centre, where it was.
 */

import { createEffect, createMemo, createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { FIT_MAX_K, MIN_READABLE_K, NODE_H, NODE_W, type PaneShape } from "./atlas-layout";
import {
    fitView,
    gapSpread,
    keepInPlace,
    localPoint,
    nearestPlaced,
    pointerScale,
    wheelFactor,
    zoomAt,
    zoomCentered,
    type Viewport,
} from "./atlas-viewport";

/** What the pane needs from the map to fit it. */
export interface MapSize {
    width: number;
    height: number;
    /** Where each node stands (its top left), for a zoom to keep the one under the pointer in place. */
    nodes?: ReadonlyMap<string, { x: number; y: number }>;
}

export interface AtlasPanZoom {
    view: Accessor<Viewport>;
    /** The pane's shape the layout is formed for: its size rounded, and slow to follow a resize. */
    shape: Accessor<PaneShape>;
    /** The share of its gaps the layout keeps: 1 in the fitted view, less once zoomed in past it. */
    spread: Accessor<number>;
    /** The element the pointer events are on. */
    setViewport: (el: HTMLDivElement) => void;
    fit: () => void;
    zoomBy: (factor: number) => void;
    onWheel: (e: WheelEvent) => void;
    onPointerDown: (e: PointerEvent) => void;
    onPointerMove: (e: PointerEvent) => void;
    onPointerUp: (e: PointerEvent) => void;
}

const FIT = { min: MIN_READABLE_K, max: FIT_MAX_K };
/** The size the layout is formed for is rounded to this, so a drag of a pixel or two changes nothing. */
const SHAPE_STEP = 64;
/** How long a resize must rest before the layout is formed again for it. */
const SHAPE_SETTLE_MS = 150;

/** A press on one of these is not on the map's background: it neither pans nor draws a selection box. */
export const NOT_BACKGROUND = ".atlas-node, .atlas-fold, .atlas-card";

/**
 * The size of the pane's `viewport` as it is, and the shape the layout is formed for: that size rounded, and slow
 * to follow a resize.
 */
function usePaneMeasure(viewport: () => HTMLDivElement | undefined) {
    const [paneSize, setPaneSize] = createSignal({ w: 0, h: 0 });
    const [shape, setShape] = createSignal<PaneShape>({ w: 0, h: 0 });
    let settle: ReturnType<typeof setTimeout> | undefined;
    const settleShape = (w: number, h: number) => {
        const next = { w: Math.round(w / SHAPE_STEP) * SHAPE_STEP, h: Math.round(h / SHAPE_STEP) * SHAPE_STEP };
        if (settle) clearTimeout(settle);
        // The first measure forms the layout at once, so the first paint is fitted; a resize waits to settle.
        if (shape().w === 0) setShape(next);
        else
            settle = setTimeout(
                () => setShape((cur) => (cur.w === next.w && cur.h === next.h ? cur : next)),
                SHAPE_SETTLE_MS
            );
    };
    onMount(() => {
        const el = viewport();
        if (!el) return;
        const measure = () => {
            setPaneSize({ w: el.clientWidth, h: el.clientHeight });
            settleShape(el.clientWidth, el.clientHeight);
        };
        measure();
        const ro = typeof ResizeObserver === "function" ? new ResizeObserver(measure) : null;
        ro?.observe(el);
        onCleanup(() => {
            ro?.disconnect();
            if (settle) clearTimeout(settle);
        });
    });
    return { paneSize, shape };
}

/**
 * `size` is the map's size, `mode` the layout shown (a change of layout is a new picture: it fits again), and
 * `onBackgroundPress` runs when a press starts a pan (the card closes).
 */
export function useAtlasPanZoom(
    size: Accessor<MapSize>,
    mode: Accessor<string>,
    onBackgroundPress: () => void
): AtlasPanZoom {
    let viewport: HTMLDivElement | undefined;
    const { paneSize, shape } = usePaneMeasure(() => viewport);
    const [view, setView] = createSignal<Viewport>({ x: 16, y: 16, k: 1 });
    const [touched, setTouched] = createSignal(false);
    /** The scale of the last fitted view: what a zoom in is measured from. */
    const [fitK, setFitK] = createSignal(1);
    // The fitted view always has the full gaps, so the size it is fitted to does not depend on its own scale.
    const spread = createMemo(() => (touched() ? gapSpread(fitK(), view().k) : 1));

    const fitNow = () => {
        const fitted = fitView(size().width, size().height, paneSize().w, paneSize().h, FIT);
        setFitK(fitted.k);
        setView(fitted);
    };
    const fit = () => {
        setTouched(false);
        fitNow();
    };
    createEffect(() => {
        if (!touched() && paneSize().w > 0) fitNow();
    });
    let lastMode = mode();
    createEffect(() => {
        if (mode() !== lastMode) {
            lastMode = mode();
            setTouched(false);
        }
    });

    const moveView = (next: Viewport) => {
        setTouched(true);
        setView(next);
    };
    /** A zoom to `next` that keeps the node nearest the map point `at` where it was, though the gaps closed or opened. */
    const zoomKeeping = (next: Viewport, at: { x: number; y: number }) => {
        const before = size().nodes;
        const key = before && nearestPlaced(before, at.x - NODE_W / 2, at.y - NODE_H / 2);
        const was = key ? before!.get(key) : undefined;
        moveView(next);
        const now = key ? size().nodes?.get(key) : undefined;
        if (was && now && (was.x !== now.x || was.y !== now.y)) setView(keepInPlace(view(), was, now));
    };
    const centred = (factor: number) =>
        zoomCentered(view(), factor, size().width, size().height, paneSize().w, paneSize().h);
    // The drag's start on screen and in the map, and the screen pixels one map pixel is (the pane zoom).
    let pan: { id: number; sx: number; sy: number; ox: number; oy: number; kx: number; ky: number } | null = null;
    return {
        view,
        shape,
        spread,
        setViewport: (el) => (viewport = el),
        fit,
        zoomBy: (factor) => {
            moveView(centred(factor));
            // The map is a new size at the new scale (its gaps closed or opened): centred again, as it now is.
            setView(centred(1));
        },
        onWheel: (e) => {
            // Ctrl or Cmd with the wheel is the pane zoom (the window's handler steps it), not the map's.
            if (e.ctrlKey || e.metaKey) return;
            e.preventDefault();
            const at = localPoint(e.clientX, e.clientY, viewport!.getBoundingClientRect(), {
                w: viewport!.clientWidth,
                h: viewport!.clientHeight,
            });
            const v = view();
            zoomKeeping(zoomAt(v, at.x, at.y, wheelFactor(e)), { x: (at.x - v.x) / v.k, y: (at.y - v.y) / v.k });
        },
        onPointerDown: (e) => {
            if (e.button !== 0 || (e.target as HTMLElement).closest(NOT_BACKGROUND)) return;
            const { sx: kx, sy: ky } = pointerScale(viewport!.getBoundingClientRect(), {
                w: viewport!.clientWidth,
                h: viewport!.clientHeight,
            });
            pan = { id: e.pointerId, sx: e.clientX, sy: e.clientY, ox: view().x, oy: view().y, kx, ky };
            viewport?.setPointerCapture(e.pointerId);
            onBackgroundPress();
        },
        onPointerMove: (e) => {
            if (!pan || e.pointerId !== pan.id) return;
            moveView({ ...view(), x: pan.ox + (e.clientX - pan.sx) / pan.kx, y: pan.oy + (e.clientY - pan.sy) / pan.ky });
        },
        onPointerUp: (e) => {
            if (pan?.id === e.pointerId) pan = null;
        },
    };
}
