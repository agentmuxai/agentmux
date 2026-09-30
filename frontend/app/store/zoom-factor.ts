// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The per-pane zoom range and the one way to read it from block meta. A leaf
// module, so view models can import it without pulling in zoom.ts's store,
// RPC and layout dependencies.

export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 2.0;
export const DEFAULT_ZOOM = 1.0;

export function clampZoom(factor: number): number {
    return Math.min(Math.max(factor, MIN_ZOOM), MAX_ZOOM);
}

/** A pane's `term:zoom`, clamped; DEFAULT_ZOOM when unset or not a number. */
export function readZoom(meta: { "term:zoom"?: unknown } | null | undefined): number {
    const z = meta?.["term:zoom"];
    if (typeof z !== "number" || isNaN(z)) return DEFAULT_ZOOM;
    return clampZoom(z);
}
