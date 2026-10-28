// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { onCleanup, type JSX } from "solid-js";
import { getApi } from "@/app/store/app-api";
import type { BrowserViewModel } from "./browser-model";
import type { PaneRect } from "./use-pane-rect-sync";

export interface DragSnapshot {
    src: string;
    style: JSX.CSSProperties;
}

// A prewarmed snapshot older than this is re-taken rather than reused: the
// press that started it didn't become a drag, and the page has moved on.
const PREWARM_MAX_AGE_MS = 3000;

// A press this soon after another's prewarm reuses it: clicking around a pane
// header is not a reason to capture every page again.
const PREWARM_REUSE_MS = 1500;

// A capture still unanswered after this is given up on. A healthy one takes
// 50-90ms; a page that can't paint (hidden, busy, frozen) answers only when the
// host's own timeouts run out, tens of seconds, holding one of the page's six
// connections to the host the whole time. Every other host command (pane rects,
// overlay clips, focus) queues behind those, so panes sat where the last rect
// put them while the layout moved on.
const CAPTURE_TIMEOUT_MS = 2000;

// After a capture fails or times out, this pane takes no snapshot for this long.
const FAILURE_BACKOFF_MS = 10_000;

// Captures in flight at once across every pane in the window.
const MAX_CONCURRENT_CAPTURES = 2;

/** Runs at most `max` jobs at once; the rest wait their turn, in order. */
export function createSlotGate(max: number): <T>(job: () => Promise<T>) => Promise<T> {
    let active = 0;
    const waiting: Array<() => void> = [];
    return async (job) => {
        if (active >= max) await new Promise<void>((go) => waiting.push(go));
        else active++;
        try {
            return await job();
        } finally {
            // The slot goes straight to the next in line, or is freed.
            const next = waiting.shift();
            if (next) next();
            else active--;
        }
    };
}

const captureGate = createSlotGate(MAX_CONCURRENT_CAPTURES);

/**
 * True when a snapshot's pixel size is this pane's own (±2px for rounding).
 * The host picks the page to capture by URL, so with several panes on one
 * URL it can return another pane's page; drawn into this pane's rect, that
 * showed up warped. Rejecting it shows the placeholder instead.
 */
export function snapshotFitsPane(natural: { width: number; height: number }, pane: PaneRect): boolean {
    return Math.abs(natural.width - pane.width) <= 2 && Math.abs(natural.height - pane.height) <= 2;
}

/**
 * The page snapshot the browser's drag catcher shows while the page is cut
 * away mid-drag (browser-view.tsx). Captured on pointer-down over anything
 * draggable, before the drag even starts, so it's usually ready by the time
 * the drag needs it; `take()` falls back to a fresh capture.
 */
export function useDragSnapshot(params: {
    model: BrowserViewModel;
    placeholderRef: () => HTMLDivElement | undefined;
    paneRect: () => PaneRect;
    paneCreated: () => boolean;
    /** The page can't paint or answer a capture now: its tab is hidden, or it is asleep or frozen
     *  (a capture would thaw a frozen page). No snapshot is taken. */
    unavailable?: () => boolean;
    diag: (msg: string) => void;
}): { take: () => Promise<DragSnapshot | null> } {
    const { model, placeholderRef, paneRect, paneCreated, diag } = params;
    let failedUntil = 0;
    let inflight: Promise<DragSnapshot | null> | null = null;

    const capture = async (): Promise<DragSnapshot | null> => {
        const pr = paneRect();
        if (!paneCreated() || model.closed || pr.width <= 0 || pr.height <= 0) return null;
        if (params.unavailable?.() || Date.now() < failedUntil) return null;
        // One capture per pane at a time, queued or running: a press while one is out shares it.
        if (inflight) return inflight;
        const mine = captureGate(() => captureOnce()).finally(() => {
            if (inflight === mine) inflight = null;
        });
        inflight = mine;
        return mine;
    };

    const captureOnce = async (): Promise<DragSnapshot | null> => {
        // Waiting for a slot can take a while: the pane may have gone since, or an earlier capture failed.
        if (!paneCreated() || model.closed || params.unavailable?.() || Date.now() < failedUntil) return null;
        const abort = new AbortController();
        const timer = setTimeout(() => abort.abort(), CAPTURE_TIMEOUT_MS);
        try {
            const data = await getApi().browserPanes.screenshot(model.blockId, {
                format: "jpeg",
                quality: 80,
                signal: abort.signal,
            });
            clearTimeout(timer);
            if (!data?.png_base64) return null;
            const src = `data:image/jpeg;base64,${data.png_base64}`;
            const img = new Image();
            img.src = src;
            await img.decode();
            const now = paneRect();
            if (!snapshotFitsPane({ width: img.naturalWidth, height: img.naturalHeight }, now)) {
                diag(`[drag-snapshot] size ${img.naturalWidth}x${img.naturalHeight} != pane ${now.width}x${now.height}; dropped`);
                return null;
            }
            const box = placeholderRef()?.getBoundingClientRect();
            if (!box) return null;
            // On the native page's own DPR-rounded rect, like the freeze
            // frame (use-freeze-frame.ts), so the swap doesn't shift.
            const dpr = window.devicePixelRatio || 1;
            return {
                src,
                style: {
                    left: `${now.x / dpr - box.x}px`,
                    top: `${now.y / dpr - box.y}px`,
                    width: `${now.width / dpr}px`,
                    height: `${now.height / dpr}px`,
                },
            };
        } catch (err) {
            clearTimeout(timer);
            failedUntil = Date.now() + FAILURE_BACKOFF_MS;
            diag(`[drag-snapshot] capture failed: ${err}`);
            return null;
        }
    };

    let prewarmed: { at: number; snapshot: Promise<DragSnapshot | null> } | null = null;
    const onPointerDown = (e: PointerEvent) => {
        if (e.button !== 0) return;
        // pragmatic-dnd marks every draggable() element draggable="true":
        // pane headers, Pane Tab pills, Window Tabs.
        if (!(e.target instanceof Element) || !e.target.closest('[draggable="true"]')) return;
        if (prewarmed && Date.now() - prewarmed.at < PREWARM_REUSE_MS) return;
        prewarmed = { at: Date.now(), snapshot: capture() };
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    onCleanup(() => document.removeEventListener("pointerdown", onPointerDown, true));

    return {
        take: () => {
            const held = prewarmed;
            prewarmed = null;
            if (held && Date.now() - held.at < PREWARM_MAX_AGE_MS) return held.snapshot;
            return capture();
        },
    };
}
