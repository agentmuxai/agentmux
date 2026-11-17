// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The per-pane zoom range and the one way to read it from block meta. A leaf
// module, so view models can import it without pulling in zoom.ts's store,
// RPC and layout dependencies.

import { createSignal } from "solid-js";

export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 2.0;
export const DEFAULT_ZOOM = 1.0;

// A block whose content is drawn at the zoom (a PDF) can go further in than the shared top. Reactive,
// so a pane that read the level before its viewer registered reads it again once it has.
const blockMaxes = new Map<string, number>();
const [rangeVersion, setRangeVersion] = createSignal(0);
export function setZoomMax(blockId: string, max: number): () => void {
    blockMaxes.set(blockId, max);
    setRangeVersion((v) => v + 1);
    return () => {
        if (blockMaxes.get(blockId) !== max) return;
        blockMaxes.delete(blockId);
        setRangeVersion((v) => v + 1);
    };
}
/** The most `blockId` zooms in to; the shared top for a block that did not ask for more. */
export function zoomMaxOf(blockId?: string): number {
    rangeVersion();
    return (blockId && blockMaxes.get(blockId)) || MAX_ZOOM;
}

export function clampZoom(factor: number, blockId?: string): number {
    return Math.min(Math.max(factor, MIN_ZOOM), zoomMaxOf(blockId));
}

/** The block meta key of a pane's own zoom; a part of a pane (store/zoom-parts.ts) keeps its own key. */
export const PANE_ZOOM_KEY = "term:zoom";

/**
 * The meta write for a pane's zoom: clamped, rounded to 0.01, and `null` (unset) at 100% so an unzoomed
 * pane carries no `term:zoom`. A part's key holds `1` at 100% instead: "set" and "never set" stay
 * apart, which the first-start seeding (`seedZoomParts`) relies on.
 */
export function paneZoomPatch<K extends string = typeof PANE_ZOOM_KEY>(
    level: number,
    blockId?: string,
    key: K = PANE_ZOOM_KEY as K
): { [P in K]: number | null } {
    const next = clampZoom(Math.round(level * 100) / 100, blockId);
    const atDefault = Math.abs(next - DEFAULT_ZOOM) < 0.005;
    return { [key]: atDefault ? (key === PANE_ZOOM_KEY ? null : DEFAULT_ZOOM) : next } as { [P in K]: number | null };
}

/** The one way to save a pane's zoom (`paneZoomPatch`). Returns the level written. */
export function writePaneZoom<K extends string = typeof PANE_ZOOM_KEY>(
    setMeta: (meta: { [P in K]: number | null }) => unknown,
    level: number,
    blockId?: string,
    key: K = PANE_ZOOM_KEY as K
): number {
    const patch = paneZoomPatch(level, blockId, key);
    void setMeta(patch);
    return patch[key] ?? DEFAULT_ZOOM;
}

/** A pane's zoom under `key` (`term:zoom` unless a part's), clamped; DEFAULT_ZOOM when unset or not a number. */
export function readZoom(meta: object | null | undefined, blockId?: string, key: string = PANE_ZOOM_KEY): number {
    const z = (meta as Record<string, unknown> | null | undefined)?.[key];
    if (typeof z !== "number" || isNaN(z)) return DEFAULT_ZOOM;
    return clampZoom(z, blockId);
}
