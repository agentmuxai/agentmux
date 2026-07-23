// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { invokeCommand } from "@/app/platform/ipc";
import { FLOATER_EDGE_RESIZE_BORDER } from "@/app/workspace/floater-resize";
import { registerPaneRect, unregisterPaneRect } from "@/app/platform/pane-rect-registry";
import { paneReflowActive, notifyPaneReflow } from "@/app/platform/pane-anim";
import type { BrowserViewModel } from "./browser-model";

export interface PaneRect {
    x: number;
    y: number;
    width: number;
    height: number;
}

export interface PaneRectSync {
    paneCreated: () => boolean;
    createPane: (url: string) => Promise<void>;
    syncPosition: () => void;
    paneRect: () => PaneRect;
}

/**
 * Syncs the native browser-pane HWND's position/size to this pane's
 * placeholder div, and owns pane creation. A native browser-pane HWND
 * can't be moved by CSS, so this polls (ResizeObserver + a safety-net
 * interval) and pushes `browser_pane_resize` whenever the placeholder's
 * rect changes, plus a settle loop after layout reflows
 * (docs/specs/SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md).
 */
export function usePaneRectSync(params: {
    model: BrowserViewModel;
    placeholderRef: () => HTMLDivElement | undefined;
    windowLabel: string;
    diag: (msg: string) => void;
}): PaneRectSync {
    const { model, placeholderRef, windowLabel, diag } = params;

    let resizeObserver: ResizeObserver | null = null;
    let positionInterval: ReturnType<typeof setInterval> | null = null;
    // Last rect we actually sent to the host. syncPosition compares against
    // this and skips the IPC when nothing changed — without this gate the
    // safety-net interval fired browser_pane_resize 5x/sec even when the
    // pane was steady, visible as a 200ms DOM blink on every tick.
    let lastSentRect: PaneRect | null = null;
    // SolidJS signal — must be reactive so <Show when={!paneCreated()}> re-runs
    // when the pane is created and hides the empty-state placeholder.
    const [paneCreated, setPaneCreated] = createSignal(false);

    // getBoundingClientRect() returns CSS pixels (device-INdependent); CEF
    // and Win32 SetWindowPos expect physical / device pixels. Multiply by
    // devicePixelRatio to convert. On HiDPI displays (dpr > 1) the pane
    // would be mispositioned/missized without this.
    const paneRect = (): PaneRect => {
        const r = placeholderRef()!.getBoundingClientRect();
        const dpr = window.devicePixelRatio || 1;
        let x = Math.round(r.x * dpr);
        const y = Math.round(r.y * dpr);
        let width = Math.round(r.width * dpr);
        let height = Math.round(r.height * dpr);
        // Floating browser pane: the floater's frontend DOM owns an invisible
        // edge grab band for edge-resize, but this pane's web-content child is
        // a separate OS window layered on top of it — so inset the child by the
        // band depth on the three window-edge sides (left/right/bottom; the top
        // edge is over the 33px header, already frontend) to expose the band.
        // SPEC_FLOATING_PANE_EDGE_RESIZE.
        if (windowLabel.startsWith("floating-")) {
            const b = Math.round(FLOATER_EDGE_RESIZE_BORDER * dpr);
            x += b;
            width = Math.max(1, width - 2 * b);
            height = Math.max(1, height - b);
        }
        return { x, y, width, height };
    };

    /** CSS-pixel rect (same coordinate space as `getBoundingClientRect`
     *  on overlay elements). Stored in `pane-rect-registry` so
     *  `sendClip` can short-circuit when no overlay intersects a pane. */
    const paneRectCss = () => {
        const r = placeholderRef()!.getBoundingClientRect();
        return {
            x: Math.round(r.x),
            y: Math.round(r.y),
            w: Math.round(r.width),
            h: Math.round(r.height),
        };
    };

    const syncPosition = () => {
        if (!placeholderRef() || !paneCreated() || model.closed) return;
        const rect = paneRect();
        if (
            lastSentRect &&
            lastSentRect.x === rect.x &&
            lastSentRect.y === rect.y &&
            lastSentRect.width === rect.width &&
            lastSentRect.height === rect.height
        ) {
            return;
        }
        lastSentRect = rect;
        invokeCommand("browser_pane_resize", {
            block_id: model.blockId,
            ...rect,
        }).catch(() => {});
        // Keep the overlay-clip short-circuit registry in sync with the
        // host's actual HWND rect. Cheap (two property reads + a Map write).
        registerPaneRect(model.blockId, paneRectCss());
    };

    // Native browser-pane HWND settle on a layout change. `notifyPaneReflow()`
    // opens a short window during which we re-sample this pane's placeholder
    // rect per frame and push it to the host (syncPosition dedupes, so
    // unchanged frames are free).
    let reflowRAF: number | null = null;
    const sampleReflowFrame = () => {
        syncPosition();
        if (paneReflowActive()) {
            reflowRAF = requestAnimationFrame(sampleReflowFrame);
        } else {
            // One final settle frame so the HWND lands exactly on the final
            // rect even if the last tick fired slightly early.
            reflowRAF = null;
            syncPosition();
        }
    };
    createEffect(() => {
        if (paneReflowActive() && reflowRAF == null) {
            reflowRAF = requestAnimationFrame(sampleReflowFrame);
        }
    });

    const createPane = async (url: string) => {
        if (!placeholderRef()) return;
        try {
            diag(`createPane url=${JSON.stringify(url)} window_label=${windowLabel}`);
            await invokeCommand("browser_pane_create", {
                block_id: model.blockId,
                url: url || "about:blank",
                window_label: windowLabel,
                ...paneRect(),
            });
            setPaneCreated(true);
            diag(`paneCreated=true`);
            // The HWND is now live — open a fresh settle window in case the
            // layout changed while the async create was in-flight.
            notifyPaneReflow();
            registerPaneRect(model.blockId, paneRectCss());
            // NOTE: does NOT call model.onLoad() here. Real load-finished
            // comes from the browser-pane-nav-state listener in
            // browser-model.ts. See
            // docs/specs/SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md §4.2.
        } catch (e) {
            model.onError(`Failed to create browser pane: ${e}`);
        }
    };

    onMount(() => {
        const ph = placeholderRef();
        if (ph) {
            resizeObserver = new ResizeObserver(syncPosition);
            resizeObserver.observe(ph);
            positionInterval = setInterval(syncPosition, 200);
        }
        // macOS/Linux: after a JS-driven drag moves the floating pane window,
        // paneRect() returns the same client coords (unchanged by window
        // movement), so syncPosition's dedupe guard skips the re-send.
        // floating-pane-workspace.tsx dispatches "floating-pane-js-drag-ended"
        // after every JS-driven drag so we can clear the dedupe guard here.
        if (windowLabel.startsWith("floating-")) {
            const onJsDragEnded = (ev: Event) => {
                const detail = (ev as CustomEvent<{ label: string }>).detail;
                if (detail?.label !== windowLabel) return;
                lastSentRect = null;
                syncPosition();
            };
            window.addEventListener("floating-pane-js-drag-ended", onJsDragEnded);
            onCleanup(() => window.removeEventListener("floating-pane-js-drag-ended", onJsDragEnded));
        }
        const url = model.urlAtom();
        if (url) createPane(url);
    });

    onCleanup(() => {
        diag(`view-unmount paneCreated=${paneCreated()}`);
        // Drop from the overlay-clip short-circuit registry FIRST so a late
        // sendClip() doesn't see a stale rect for the closed pane.
        unregisterPaneRect(model.blockId);
        // Fire close IPC BEFORE disconnecting observers — the IPC flips the
        // backend pane to Closing, so any in-flight resize/focus/nav calls
        // that haven't reached the backend yet get no-op'd there instead of
        // racing a mid-destruction HWND. See SPEC_BROWSER_PANE_LIFECYCLE.md §5.
        if (paneCreated()) {
            invokeCommand("browser_pane_close", { block_id: model.blockId, window_label: windowLabel }).catch(() => {});
        }
        resizeObserver?.disconnect();
        if (positionInterval) {
            clearInterval(positionInterval);
            positionInterval = null;
        }
        if (reflowRAF != null) {
            cancelAnimationFrame(reflowRAF);
            reflowRAF = null;
        }
    });

    return { paneCreated, createPane, syncPosition, paneRect };
}
