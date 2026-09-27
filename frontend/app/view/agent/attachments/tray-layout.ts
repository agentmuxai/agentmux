// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export const TILE_PX = 64;
export const TILE_GAP_PX = 6;

/**
 * How many tiles fit in one collapsed row of `widthPx`, and how many spill
 * into the "+N" tile. When they don't all fit, the last slot becomes the
 * "+N" tile, so one fewer real tile shows. Spec §5.2.
 */
export function trayLayout(count: number, widthPx: number): { visible: number; overflow: number } {
    if (count <= 0) return { visible: 0, overflow: 0 };
    const fit = Math.max(1, Math.floor((Math.max(0, widthPx) + TILE_GAP_PX) / (TILE_PX + TILE_GAP_PX)));
    if (count <= fit) return { visible: count, overflow: 0 };
    const visible = Math.max(0, fit - 1);
    return { visible, overflow: count - visible };
}

/** Warning level for "N / max" and "size / max": ≥ 80% warns, over errors. */
export function limitLevel(value: number, max: number): "ok" | "warn" | "over" {
    if (max <= 0) return "ok";
    if (value > max) return "over";
    if (value >= max * 0.8) return "warn";
    return "ok";
}
