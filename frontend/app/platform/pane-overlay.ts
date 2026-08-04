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
import { anyPaneIntersects, paneCount } from "@/app/platform/pane-rect-registry";
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

// Per-frame coalescing: every overlay add/remove/resize used to fire its
// own IPC synchronously. A single menu hover transition emits 2–3 rect
// changes in the same microtask checkpoint (Show toggle off, Show toggle
// on, overflow-correction re-write), so we'd send 2–3 separate
// SetWindowRgn fan-outs for one user gesture. Batching into one rAF tick
// is the dominant win — see discussion #1097, fix #2.
//
// Visible-for-tests scheduling state: a Solid signal would create a
// reactive owner dependency on whichever component triggered the first
// scheduleSendClip call, which isn't what we want — the schedule is
// global.
let clipScheduled = false;
let lastDispatchedKey = "";
// Serializes async flushes: a flush may await pane freeze-frames before
// dispatching the hide IPC (see flushClip), and a second rAF-scheduled
// flush during that window must not overtake it — out-of-order clip IPCs
// would let a stale hide clobber a newer clear.
let flushChain: Promise<void> = Promise.resolve();
// Upper bound on how long a pending hide waits for pane freeze-frames.
// A slow or failed capture may only DELAY the hide, never block it.
const FREEZE_WAIT_CAP_MS = 300;

function rectsKey(rects: readonly OverlayRect[]): string {
    if (rects.length === 0) return "";
    let s = "";
    for (const r of rects) s += `${r.x},${r.y},${r.w},${r.h}|`;
    return s;
}

function sendClip(): void {
    if (clipScheduled) return;
    clipScheduled = true;
    // rAF is the correct queue: the user gesture's DOM mutations have
    // already committed by the time the rAF fires, so we send the
    // settled state instead of the in-flight one.
    requestAnimationFrame(() => {
        clipScheduled = false;
        flushChain = flushChain.then(() => flushClip()).catch(() => {});
    });
}

async function flushClip(): Promise<void> {
    const rects: OverlayRect[] = [];
    for (const r of overlayRects.values()) rects.push(r);
    for (const r of autoOverlayRects.values()) {
        if (r.w > 0 && r.h > 0) rects.push(r);
    }

    // Short-circuits — see discussion #1097, fix #1.
    //
    // (a) Zero panes alive in this window? The Rust handler would no-op
    //     anyway after iterating an empty pane list, but we still pay
    //     the HTTP round-trip + JSON serialize/parse + main-thread
    //     await. Skip entirely.
    // (b) Overlay rects don't intersect any pane? Common case for the
    //     hamburger menu, statusbar popovers, tooltips in a workspace
    //     whose layout has the panes elsewhere. No clip work needed
    //     and no holes to punch.
    //
    // CRITICAL: when transitioning from "previously intersecting" to
    // "now does not intersect" (e.g. an overlay moves off the pane,
    // or its rect shrinks to non-overlapping), we MUST dispatch a
    // clearing IPC — otherwise Rust keeps the prior clip applied and
    // the pane shows a transparent hole where the overlay used to be.
    // The "previously intersecting" state is whether
    // `lastDispatchedKey` is non-empty (= we last dispatched some
    // non-empty rect set to Rust).
    const intersects = rects.length > 0 && paneCount() > 0 && rects.some(anyPaneIntersects);
    const wasNonEmpty = lastDispatchedKey !== "";
    let needIpc: boolean;
    if (intersects) {
        needIpc = true;
    } else if (wasNonEmpty) {
        // We last sent a non-empty clip; now nothing intersects (either
        // overlay rect changed or the panes moved). Dispatch a CLEARING
        // IPC with empty rects so Rust resets each pane's region.
        needIpc = true;
    } else {
        // Nothing intersected before, nothing intersects now → no-op.
        needIpc = false;
    }

    if (!needIpc) {
        // No IPC fired → don't touch lastDispatchedKey. The dedup gate
        // remains correctly anchored to whatever Rust last received.
        return;
    }
    // What we'll actually send: the intersecting set when there IS an
    // intersection, else an explicit empty set to clear.
    //
    // Convert CSS px → physical px HERE. The host computes each pane's
    // geometry from GetWindowRect (physical px) and subtracts these overlay
    // rects directly (browser_panes.rs::set_pane_overlay_clip — no DPI
    // scaling on that side). A CSS-px rect therefore punches the airspace
    // hole in the wrong place and at the wrong size on any display scale
    // != 100%: black voids where the hole misses the overlay, and the pane
    // covering the overlay where the hole is too small (the "offset menus /
    // black spots / hidden" airspace bug — see
    // docs/analysis/ANALYSIS_BROWSER_PANE_AIRSPACE_ARCHITECTURE_2026_05_30.md).
    // Mirror browser-view.tsx::paneRect's `Math.round(v * dpr)` convention
    // EXACTLY so the hole and the pane HWND share rounding and never leave a
    // 1px seam. The intersection gate above stays in CSS px because the
    // pane-rect registry it tests against is CSS px.
    const dpr = window.devicePixelRatio || 1;
    const rectsToSend: OverlayRect[] = (intersects ? rects : []).map((r) => ({
        x: Math.round(r.x * dpr),
        y: Math.round(r.y * dpr),
        w: Math.round(r.w * dpr),
        h: Math.round(r.h * dpr),
    }));
    const sendKey = rectsKey(rectsToSend);
    // Identical-rect deduplication — if the menu closes-then-reopens
    // exactly the same overlay rect within one tick, skip the redundant
    // IPC. Most observed-rect change cycles aren't identical, so this
    // is a small bonus on top of the rAF coalesce.
    if (sendKey === lastDispatchedKey) return;
    lastDispatchedKey = sendKey;
    const window_label = currentWindowLabel();
    // Mirror every dispatched clip to the DOM (CSS px, pre-DPR — matching
    // getBoundingClientRect space) so pane views can react locally. On
    // macOS/Linux the host responds to an intersecting clip by hiding the
    // WHOLE pane NSWindow (no SetWindowRgn equivalent), which exposes the
    // bare placeholder; browser-view.tsx listens for this event to show a
    // freeze-frame of the pane instead of a blank surface. Shares the
    // dedup gates above, so it fires exactly when the host's clip state
    // actually changes.
    //
    // ORDER MATTERS: the event fires BEFORE the hide IPC, and handlers may
    // register readiness promises via `detail.wait(promise)`. When hiding
    // (intersects), we await those (capped at FREEZE_WAIT_CAP_MS) so each
    // pane's freeze-frame is painted UNDER the still-visible native pane
    // before the hide lands — the swap from live pixels to the identical
    // snapshot is then seamless instead of flashing the bare placeholder
    // for the length of a screenshot roundtrip. When clearing, no handler
    // registers a wait and the IPC dispatches immediately.
    const waits: Promise<unknown>[] = [];
    window.dispatchEvent(
        new CustomEvent("pane-overlay-clip-changed", {
            detail: {
                rects: intersects ? rects.slice() : [],
                wait: (p: Promise<unknown>) => waits.push(p),
            },
        }),
    );
    if (intersects && waits.length > 0) {
        await Promise.race([
            Promise.all(waits).catch(() => {}),
            new Promise((resolve) => setTimeout(resolve, FREEZE_WAIT_CAP_MS)),
        ]);
    }
    invokeCommand("browser_panes_set_overlay_clip", { rects: rectsToSend, window_label }).catch(
        () => {},
    );
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
 * `window.resize`, and re-reads it whenever a `ResizeObserver` reports
 * the element's box changing — including the collapse to 0 when a
 * tab-scoped overlay's tab is hidden with `display:none` (no `resize`
 * fires for that, so without the observer the backend keeps a stale clip
 * that punches a hole through the newly active tab's panes). It
 * deregisters on unmount. Overlays that move without any size change
 * (dropped anchors) may still need an `IntersectionObserver` layered on.
 *
 * Safe to nest — each call registers its own rect, the union is applied.
 * No-op on platforms without native pane HWNDs (backend IPC is a no-op).
 */
export function usePaneOverlay(getEl: Accessor<HTMLElement | null | undefined>): void {
    const id = nextOverlayId++;
    const update = (): void => {
        const el = getEl();
        if (!el) return;
        const rect = rectFromElement(el);
        // A collapsed box (e.g. the overlay's tab hidden via display:none)
        // contributes no hole — drop the entry rather than send a 0×0 rect.
        if (rect.w > 0 && rect.h > 0) {
            overlayRects.set(id, rect);
        } else {
            overlayRects.delete(id);
        }
        sendClip();
    };
    let observer: ResizeObserver | undefined;
    let styleObserver: MutationObserver | undefined;
    onMount(() => {
        update();
        window.addEventListener("resize", update);
        const el = getEl();
        if (el && typeof ResizeObserver !== "undefined") {
            // A tab-scoped modal overlay is hidden via `display:none` when
            // its tab goes inactive — the box collapses to 0 but no `resize`
            // fires, so a ResizeObserver is needed to drop the now-stale rect
            // (and to restore it when the tab is shown again).
            observer = new ResizeObserver(() => update());
            observer.observe(el);
        }
        if (el && typeof MutationObserver !== "undefined") {
            // Floating-UI-positioned overlays (menus, dropdowns) mount at a
            // placeholder position (left:0;top:0) and get their real position
            // committed asynchronously via a style write. That's a MOVE with
            // no size change — the ResizeObserver stays silent — so without
            // watching `style`, the registered rect stays wherever the mount
            // race left it. Whether the clip landed at the right place was
            // literally a race between RO's initial callback and floating-ui's
            // position commit (the long-observed "works sometimes, breaks
            // again" flakiness). Mirrors pane-overlay-auto.ts's style watcher.
            styleObserver = new MutationObserver(() => update());
            styleObserver.observe(el, { attributes: true, attributeFilter: ["style"] });
        }
    });
    onCleanup(() => {
        window.removeEventListener("resize", update);
        observer?.disconnect();
        styleObserver?.disconnect();
        if (overlayRects.delete(id)) {
            sendClip();
        }
    });
}
