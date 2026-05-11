// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Native browser pane HWNDs composite above DOM at the Win32 level
// regardless of CSS z-index (the "airspace" problem). Any DOM overlay
// that can overlap a pane area — widget-bar dropdown, popovers, modals —
// will appear *behind* the pane content unless we cut a transparent hole
// through the pane HWND.
//
// Strategy: the backend applies a `SetWindowRgn` clip to each pane HWND
// that subtracts the union of all currently-open overlay rectangles. The
// pane renders normally outside the overlay region; inside it, the HWND
// is transparent so the DOM overlay painted at the same screen position
// shows through. Empty overlay set → clip cleared → full pane visibility.
//
// See `BROWSER_PANE_Z_ORDER_FOCUS_REPORT.md` Issue 1 for the full diagnosis.

import { invokeCommand } from "@/app/platform/ipc";
import { onCleanup, onMount, type Accessor } from "solid-js";

interface OverlayRect {
    x: number;
    y: number;
    w: number;
    h: number;
}

const overlayRects = new Map<number, OverlayRect>();
let nextOverlayId = 1;

// Second map for rects discovered automatically by the auto-clip
// service (pane-overlay-auto.ts), keyed by element ref. Hook callers
// and auto-discovery feed two maps; sendClip unions them. Existing
// hook callers do not need to migrate.
const autoOverlayRects = new Map<Element, OverlayRect>();

/**
 * Window label of the host window this frontend instance belongs to.
 * Set by the CEF launcher on each window's startup URL via `?windowLabel=`.
 * Defaults to `"main"` so single-window builds work unchanged. Backend
 * uses this to scope the overlay clip to panes owned by THIS window
 * (fixes Codex P1 on PR #544: without scoping, a modal in window B
 * would clip panes in window A).
 */
function currentWindowLabel(): string {
    try {
        return new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
    } catch {
        return "main";
    }
}

function sendClip(): void {
    const rects: OverlayRect[] = [];
    for (const r of overlayRects.values()) rects.push(r);
    for (const r of autoOverlayRects.values()) {
        if (r.w > 0 && r.h > 0) rects.push(r);
    }
    const window_label = currentWindowLabel();
    invokeCommand("browser_panes_set_overlay_clip", { rects, window_label }).catch(() => {});
}

/**
 * Auto-discovery hooks for `pane-overlay-auto.ts`. Each function
 * mutates the shared map and calls `sendClip()`. Returns true if the
 * map actually changed (so callers can skip redundant dispatches).
 */
export function __setAutoOverlayRect(el: Element, rect: OverlayRect): boolean {
    if (rect.w <= 0 || rect.h <= 0) return __deleteAutoOverlayRect(el);
    const prev = autoOverlayRects.get(el);
    if (
        prev &&
        prev.x === rect.x && prev.y === rect.y &&
        prev.w === rect.w && prev.h === rect.h
    ) return false;
    autoOverlayRects.set(el, rect);
    sendClip();
    return true;
}

export function __deleteAutoOverlayRect(el: Element): boolean {
    if (!autoOverlayRects.delete(el)) return false;
    sendClip();
    return true;
}

export function __rectFromElement(el: HTMLElement): OverlayRect {
    return rectFromElement(el);
}

function rectFromElement(el: HTMLElement): OverlayRect {
    const r = el.getBoundingClientRect();
    return {
        x: Math.round(r.left),
        y: Math.round(r.top),
        w: Math.round(r.width),
        h: Math.round(r.height),
    };
}

/**
 * Cut a transparent hole through every browser pane HWND matching the
 * given overlay element's screen rect, so the overlay renders visually
 * *over* the pane instead of being occluded by it. Call this inside any
 * component rendering a DOM overlay that could overlap a pane.
 *
 * The hook reads the element's bounding rect on mount, re-reads it on
 * `window.resize` (so viewport-sized overlays like the modal-v2 backdrop
 * follow resizes correctly), and deregisters on unmount. Overlays that
 * move independently of window resize (dropped anchors, animated
 * transitions) can layer a `ResizeObserver` / `IntersectionObserver` on
 * top and re-call the hook — for now, window resize is the only
 * observed mutation in the live surfaces.
 *
 * Safe to nest — each call registers its own rect, the union is applied.
 * No-op on platforms without native pane HWNDs (backend IPC is a no-op).
 */
export function usePaneOverlay(getEl: Accessor<HTMLElement | null | undefined>): void {
    const id = nextOverlayId++;
    const update = (): void => {
        const el = getEl();
        if (!el) return;
        overlayRects.set(id, rectFromElement(el));
        sendClip();
    };
    onMount(() => {
        update();
        window.addEventListener("resize", update);
    });
    onCleanup(() => {
        window.removeEventListener("resize", update);
        if (overlayRects.delete(id)) {
            sendClip();
        }
    });
}
