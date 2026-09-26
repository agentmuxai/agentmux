// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { getApi } from "@/app/store/app-api";
import { FLOATER_EDGE_RESIZE_BORDER } from "@/app/workspace/floater-resize";
import { registerPaneRect, unregisterPaneRect } from "@/app/platform/pane-rect-registry";
import { paneReflowActive, notifyPaneReflow } from "@/app/platform/pane-anim";
import { usePaneTabVisibility } from "@/app/block/pane-tab-visibility";
import { focusManager } from "@/app/store/focusManager";
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
 * Which mount of a block's browser view owns its native page. The host keys
 * the page by block id, and a view can mount again before an earlier mount's
 * `browser_pane_create` returns (a split, a layout rebuild, a hot reload); the
 * host then answers the newer create with `create-already-live` and the newer
 * mount adopts the page. The LATEST mount to request it owns it: an older
 * mount must not close it on unmount or as a "finished after unmount" orphan,
 * or the newer mount is left with no page — a black pane
 * (REPORT_SYSINFO_PLOT_TYPE_AND_BROWSER_PREVIEW_VM_2026_09_25.md §2.4).
 */
const nativePaneOwners = new Map<string, symbol>();

/** Releases `token`'s claim on `blockId`'s native page; true when it still
 *  owned it, i.e. the caller should close the page. */
function releaseNativePane(blockId: string, token: symbol | null): boolean {
    if (token == null || nativePaneOwners.get(blockId) !== token) return false;
    nativePaneOwners.delete(blockId);
    return true;
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
    // The tab's one visibility signal (Pane Tab contract Phase 3): the native
    // page is collapsed whenever the tab isn't "active".
    const visibility = usePaneTabVisibility(model.blockId);

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
        // Collapse whenever the tab isn't visible (`usePaneTabVisibility`):
        // its window tab isn't displayed, or it's a kept-alive tab that isn't
        // its pane's active one. The placeholder's rect can't tell:
        //
        // Inside an inactive workspace tab (content-visibility:hidden since
        // PR #3239 — workspace.tsx), the placeholder's getBoundingClientRect()
        // keeps returning its last-known REAL size — unlike the display:none
        // this replaced, which zeroed it, content-visibility:hidden does not
        // collapse a descendant with an explicit (non-content-derived) size.
        // Without this check, the safety-net interval below (every 200ms,
        // unconditional on visibility) keeps pushing that stale non-zero rect
        // to the host, which keeps the native browser-pane HWND/surface sized
        // and composited — invisible to any DOM-level fix (content-visibility,
        // pointer-events), since native panes composite ABOVE the DOM
        // entirely, independent of it. codex P1 on PR #3239.
        //
        // Same for a browser that is a kept-alive but INACTIVE pane tab
        // (pane-leaf-chrome keeps it mounted and only hides it with
        // `visibility`, which doesn't change this rect). Without this its
        // native page stayed drawn over whichever tab was active in the pane.
        const rect = visibility() !== "active" ? { x: 0, y: 0, width: 0, height: 0 } : paneRect();
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
        getApi().browserPanes.resize(model.blockId, rect).catch(() => {});
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

    // Set on unmount. `browser_pane_create` is async: if the view unmounts
    // (its tab switched away) while the create is still in flight, the
    // unmount's close is skipped (`paneCreated()` is still false), and the
    // native page would be created afterwards and never closed, left drawn
    // over whatever tab is active.
    let disposed = false;
    // This mount's claim on the block's native page (`nativePaneOwners`).
    let ownerToken: symbol | null = null;

    const createPane = async (url: string) => {
        if (!placeholderRef()) return;
        const token = Symbol(model.blockId);
        ownerToken = token;
        nativePaneOwners.set(model.blockId, token);
        try {
            diag(`createPane url=${JSON.stringify(url)} window_label=${windowLabel}`);
            await getApi().browserPanes.create(model.blockId, url || "about:blank", windowLabel, paneRect());
            if (disposed) {
                if (releaseNativePane(model.blockId, token)) {
                    diag(`createPane finished after unmount — closing the orphan`);
                    getApi().browserPanes.close(model.blockId, windowLabel).catch(() => {});
                } else {
                    diag(`createPane finished after unmount — a newer mount owns the page, leaving it open`);
                }
                return;
            }
            setPaneCreated(true);
            diag(`paneCreated=true`);
            // The HWND is now live — open a fresh settle window in case the
            // layout changed while the async create was in-flight.
            notifyPaneReflow();
            registerPaneRect(model.blockId, paneRectCss());
            // The page exists now, so it can take the keyboard: if this pane
            // is the selected one (opened with focus, e.g. `muxsh web`), move
            // the caret into it, as terminal/editor panes do on mount. A
            // giveFocus() attempted earlier (at layout insert) found no page
            // yet. claimFocusOnMount skips a background tab's pane and a
            // caret the user already put in this pane's own URL bar.
            focusManager.claimFocusOnMount(model.blockId, () => model.giveFocus());
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

    // Re-sync when the tab's visibility changes (see syncPosition): hiding
    // or showing doesn't change the placeholder's geometry, so the
    // ResizeObserver can't see it, and waiting for the 200ms interval would
    // show a stale frame.
    createEffect(() => {
        visibility();
        if (placeholderRef()) syncPosition();
    });

    onCleanup(() => {
        disposed = true;
        diag(`view-unmount paneCreated=${paneCreated()}`);
        // Drop from the overlay-clip short-circuit registry FIRST so a late
        // sendClip() doesn't see a stale rect for the closed pane.
        unregisterPaneRect(model.blockId);
        // Fire close IPC BEFORE disconnecting observers — the IPC flips the
        // backend pane to Closing, so any in-flight resize/focus/nav calls
        // that haven't reached the backend yet get no-op'd there instead of
        // racing a mid-destruction HWND. See SPEC_BROWSER_PANE_LIFECYCLE.md §5.
        if (paneCreated() && releaseNativePane(model.blockId, ownerToken)) {
            getApi().browserPanes.close(model.blockId, windowLabel).catch(() => {});
        } else if (paneCreated()) {
            diag(`view-unmount — a newer mount owns the page, leaving it open`);
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
