// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * A pane's zoom the moment it is asked for, before the backend has stored it. `term:zoom` is an RPC
 * write that the pane sees only when the update reaches the cache, so a zoom lagged and a second
 * wheel tick sent before the first came back read the old level. The store holds the level it just
 * wrote; `paneZoom` returns it in place of the meta and the stepper starts from it. It is let go once
 * the write has settled and its update had time to arrive, at once if the write was refused.
 * docs/specs/SPEC_ZOOM_ANCHOR_AND_AUDIT_2026_10_29.md §4.1.
 */

import { createSignal } from "solid-js";
import { readZoom } from "./zoom-factor";

/** How long after a write settles the level is still held, for its update to reach the cache. */
export const LOCAL_ZOOM_HOLD_MS = 1500;

const [held, setHeld] = createSignal<Readonly<Record<string, number>>>({});
/** The newest write per block: an older write settling must not let go of a newer level. */
const newest = new Map<string, number>();
const timers = new Map<string, ReturnType<typeof setTimeout>>();

/** The level just written for `blockId`, if one is held. Reactive. */
export function heldPaneZoom(blockId: string): number | undefined {
    return held()[blockId];
}

/**
 * A pane's zoom: the level just written while it is held, else what its meta says. Reactive in
 * both, so a pane's memo follows the zoom as it is asked for and the stored value after.
 */
export function paneZoom(blockId: string, meta: { "term:zoom"?: unknown } | null | undefined): number {
    return heldPaneZoom(blockId) ?? readZoom(meta, blockId);
}

/** A write of `level` is on its way. Returns the write's number, for `settlePaneZoom`. */
export function holdPaneZoom(blockId: string, level: number): number {
    const seq = (newest.get(blockId) ?? 0) + 1;
    newest.set(blockId, seq);
    clearTimeout(timers.get(blockId));
    timers.delete(blockId);
    setHeld((h) => ({ ...h, [blockId]: level }));
    return seq;
}

/** The write numbered `seq` has finished (`failed`: it was refused). Let go of the level in time. */
export function settlePaneZoom(blockId: string, seq: number, failed: boolean): void {
    if (newest.get(blockId) !== seq) return; // a newer write holds the level
    clearTimeout(timers.get(blockId));
    const release = (): void => {
        timers.delete(blockId);
        if (newest.get(blockId) !== seq) return;
        newest.delete(blockId);
        setHeld((h) => {
            const { [blockId]: _gone, ...rest } = h;
            return rest;
        });
    };
    if (failed) release();
    else timers.set(blockId, setTimeout(release, LOCAL_ZOOM_HOLD_MS));
}

/** For tests: forget every held level. */
export function resetPaneZoomForTests(): void {
    for (const t of timers.values()) clearTimeout(t);
    timers.clear();
    newest.clear();
    setHeld({});
}
