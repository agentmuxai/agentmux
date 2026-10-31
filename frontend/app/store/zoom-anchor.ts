// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Zoom that keeps what is under the pointer where it is. A pane opts in with `useZoomAnchor` and an
 * adapter (`capture` what is under a point, `restore` it). The zoom store calls `captureZoomAnchor`
 * just before a pane's zoom changes; the hook restores after the layout, and again over the next
 * frames while measurements land. The hook owns the timing, the adapter the pane's scroll rules.
 * docs/specs/SPEC_ZOOM_ANCHOR_AND_AUDIT_2026_10_29.md §4.1.
 */

import { createEffect, on, onCleanup, type Accessor } from "solid-js";

/** A point in the window, in client px. */
export interface ZoomPoint {
    clientX: number;
    clientY: number;
}

export interface ZoomAnchorAdapter<T = unknown> {
    /** What is under `point` (the pane's centre when null or outside the pane); null: nothing to hold. */
    capture(point: ZoomPoint | null): T | null;
    /** Put it back where `capture` found it. True when it already was, or there is nothing to do. */
    restore(token: T): boolean;
}

/** Zoom steps this close together (ms), with the pointer not moved, are one gesture: one anchor. */
export const GESTURE_MS = 200;
/** The pointer moved no more than this (px) is not a new anchor. */
const GESTURE_SLOP_PX = 4;
/** Frames the restore is repeated at most, while the rows are measured. */
export const SETTLE_FRAMES = 10;
/**
 * A capture older than this (ms) when the zoom changes is not for it: a pane whose zoom may not change
 * (a terminal's font size rounds to the same size) must not restore a stale point at a later change.
 */
export const STALE_MS = 2000;

interface Entry {
    adapter: ZoomAnchorAdapter;
    session?: { token: unknown; point: ZoomPoint | null; at: number };
    /** A capture was made for the next zoom change. */
    armed: boolean;
}

const entries = new Map<string, Entry>();

let pointer: ZoomPoint | null = null;
if (typeof window !== "undefined") {
    window.addEventListener(
        "pointermove",
        (e) => {
            pointer = { clientX: e.clientX, clientY: e.clientY };
        },
        { capture: true, passive: true }
    );
}

/** Where the pointer last was in this window: the keyboard's anchor, if it is over the pane. */
export function lastPointer(): ZoomPoint | null {
    return pointer;
}

const near = (a: ZoomPoint | null, b: ZoomPoint | null): boolean =>
    a == null || b == null
        ? a === b
        : Math.abs(a.clientX - b.clientX) <= GESTURE_SLOP_PX && Math.abs(a.clientY - b.clientY) <= GESTURE_SLOP_PX;

/**
 * Before `blockId`'s zoom changes: note what is under `point`. A step that follows another within
 * `GESTURE_MS` with the pointer in the same place keeps the first one's anchor, so a run of wheel
 * ticks holds one spot instead of drifting by each step's rounding.
 */
export function captureZoomAnchor(blockId: string, point: ZoomPoint | null, now: number = performance.now()): void {
    const entry = entries.get(blockId);
    if (!entry) return;
    const held = entry.session;
    if (held && now - held.at < GESTURE_MS && near(held.point, point)) {
        held.at = now;
        entry.armed = true;
        return;
    }
    const token = entry.adapter.capture(point);
    entry.session = token == null ? undefined : { token, point, at: now };
    entry.armed = token != null;
}

export interface UseZoomAnchor {
    blockId: string;
    /** The pane's zoom. A change of it, after a capture, is restored. */
    zoom: Accessor<number>;
    /** The pane's scroller: a scroll by the user ends the restore. */
    scroller: () => HTMLElement | undefined;
    /** The pane follows its latest output: nothing to hold, and the restore is dropped. */
    following: () => boolean;
    adapter: ZoomAnchorAdapter;
}

/** Keys that scroll the reader, which ends a restore in progress. */
const SCROLL_KEYS = new Set(["PageUp", "PageDown", "Home", "End", "ArrowUp", "ArrowDown", " "]);

export function useZoomAnchor(opts: UseZoomAnchor): void {
    const entry: Entry = { adapter: opts.adapter, armed: false };
    entries.set(opts.blockId, entry);
    let stop: (() => void) | undefined;
    onCleanup(() => {
        stop?.();
        if (entries.get(opts.blockId) === entry) entries.delete(opts.blockId);
    });

    createEffect(
        on(
            opts.zoom,
            () => {
                if (!entry.armed || !entry.session) return;
                entry.armed = false;
                if (performance.now() - entry.session.at > STALE_MS) return;
                stop?.();
                stop = settle(entry.session.token, opts);
            },
            { defer: true }
        )
    );
}

/** Restore now, and again each frame until two passes in a row agree, the reader scrolls or it follows. */
function settle(token: unknown, opts: UseZoomAnchor): () => void {
    const scroller = opts.scroller();
    let cancelled = false;
    let frame = 0;
    let raf = 0;
    const cancel = (): void => {
        cancelled = true;
        if (raf) cancelAnimationFrame(raf);
        raf = 0;
        end();
    };
    const onWheel = (e: WheelEvent): void => {
        if (!e.ctrlKey && !e.metaKey) cancel();
    };
    const onKey = (e: KeyboardEvent): void => {
        if (SCROLL_KEYS.has(e.key) && !e.ctrlKey && !e.metaKey) cancel();
    };
    const end = (): void => {
        scroller?.removeEventListener("wheel", onWheel);
        scroller?.removeEventListener("touchstart", cancel);
        scroller?.removeEventListener("pointerdown", cancel);
        scroller?.removeEventListener("keydown", onKey);
    };
    scroller?.addEventListener("wheel", onWheel, { passive: true });
    scroller?.addEventListener("touchstart", cancel, { passive: true });
    scroller?.addEventListener("pointerdown", cancel, { passive: true });
    scroller?.addEventListener("keydown", onKey);

    let agreed = 0;
    const pass = (): void => {
        raf = 0;
        if (cancelled) return;
        if (opts.following()) return cancel();
        agreed = opts.adapter.restore(token) ? agreed + 1 : 0;
        if (agreed >= 2 || ++frame >= SETTLE_FRAMES) return cancel();
        raf = requestAnimationFrame(pass);
    };
    pass();
    return cancel;
}
