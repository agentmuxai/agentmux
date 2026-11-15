// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The map's pan and zoom state for the Atlas pane. The map fits the pane, and follows the pane and the agents,
 * until the person pans or zooms it: from then on it stays where they put it, and Fit (or the layout toggle) takes
 * it back. The wheel zooms about the pointer, the zoom buttons zoom with the map centred in the pane, and a drag
 * on the background pans.
 */

import { createEffect, createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { fitView, localPoint, wheelFactor, zoomAt, zoomCentered, type Viewport } from "./atlas-viewport";

/** What the pane needs from the map to fit it. */
export interface MapSize {
    width: number;
    height: number;
}

export interface AtlasPanZoom {
    view: Accessor<Viewport>;
    /** The element the pointer events are on. */
    setViewport: (el: HTMLDivElement) => void;
    fit: () => void;
    zoomBy: (factor: number) => void;
    onWheel: (e: WheelEvent) => void;
    onPointerDown: (e: PointerEvent) => void;
    onPointerMove: (e: PointerEvent) => void;
    onPointerUp: (e: PointerEvent) => void;
}

/** A press on one of these is not on the map's background: it neither pans nor draws a selection box. */
export const NOT_BACKGROUND = ".atlas-node, .atlas-fold, .atlas-card";

/**
 * `size` is the map's size, `mode` the layout shown (a change of layout is a new picture: it fits again), and
 * `onBackgroundPress` runs when a press starts a pan (the card closes).
 */
export function useAtlasPanZoom(
    size: Accessor<MapSize>,
    mode: Accessor<string>,
    onBackgroundPress: () => void
): AtlasPanZoom {
    const [view, setView] = createSignal<Viewport>({ x: 16, y: 16, k: 1 });
    const [paneSize, setPaneSize] = createSignal({ w: 0, h: 0 });
    const [touched, setTouched] = createSignal(false);
    let viewport: HTMLDivElement | undefined;

    const fit = () => {
        setTouched(false);
        setView(fitView(size().width, size().height, paneSize().w, paneSize().h));
    };
    createEffect(() => {
        const s = size();
        const p = paneSize();
        if (!touched() && p.w > 0) setView(fitView(s.width, s.height, p.w, p.h));
    });
    let lastMode = mode();
    createEffect(() => {
        if (mode() !== lastMode) {
            lastMode = mode();
            setTouched(false);
        }
    });
    onMount(() => {
        if (!viewport) return;
        const measure = () => setPaneSize({ w: viewport!.clientWidth, h: viewport!.clientHeight });
        measure();
        const ro = typeof ResizeObserver === "function" ? new ResizeObserver(measure) : null;
        ro?.observe(viewport);
        onCleanup(() => ro?.disconnect());
    });

    const moveView = (next: Viewport) => {
        setTouched(true);
        setView(next);
    };
    let pan: { id: number; sx: number; sy: number; ox: number; oy: number } | null = null;
    return {
        view,
        setViewport: (el) => (viewport = el),
        fit,
        zoomBy: (factor) =>
            moveView(zoomCentered(view(), factor, size().width, size().height, paneSize().w, paneSize().h)),
        onWheel: (e) => {
            e.preventDefault();
            const at = localPoint(e.clientX, e.clientY, viewport!.getBoundingClientRect(), {
                w: viewport!.clientWidth,
                h: viewport!.clientHeight,
            });
            moveView(zoomAt(view(), at.x, at.y, wheelFactor(e)));
        },
        onPointerDown: (e) => {
            if (e.button !== 0 || (e.target as HTMLElement).closest(NOT_BACKGROUND)) return;
            pan = { id: e.pointerId, sx: e.clientX, sy: e.clientY, ox: view().x, oy: view().y };
            viewport?.setPointerCapture(e.pointerId);
            onBackgroundPress();
        },
        onPointerMove: (e) => {
            if (!pan || e.pointerId !== pan.id) return;
            moveView({ ...view(), x: pan.ox + e.clientX - pan.sx, y: pan.oy + e.clientY - pan.sy });
        },
        onPointerUp: (e) => {
            if (pan?.id === e.pointerId) pan = null;
        },
    };
}
