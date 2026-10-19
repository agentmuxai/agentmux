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
// That is the Windows path; on macOS/Linux there is no region clip, so the
// host hides a whole pane while any overlay intersects it (see flushClip).

import { getApi } from "@/app/store/app-api";
import { anyPaneIntersects, getPaneRect, paneCount } from "@/app/platform/pane-rect-registry";
import { useWindowTabDisplayed } from "@/app/workspace/window-tab-visibility";
import { createEffect, on, onCleanup, onMount, type Accessor } from "solid-js";

// Hidden window tabs and dormant pane-stack members stay mounted and keep
// their real size and position (workspace.tsx, pane-leaf-chrome.tsx), so an
// overlay inside one still has a rect over the panes of the tab on screen.
// Their containers carry these markers, and an overlay under one cuts no hole:
// docs/reports/REPORT_BROWSER_PANE_GREY_HOLE_RCA_2026_10_18.md.
/** On a window tab's container while it isn't the displayed tab (workspace.tsx). */
export const HIDDEN_TAB_ATTR = "data-window-tab-hidden";
/** On a pane-stack slot while its member isn't the active one (pane-leaf-chrome.tsx). */
export const DORMANT_SLOT_ATTR = "data-pane-dormant";
const HIDDEN_CONTAINER_SELECTOR = `[${HIDDEN_TAB_ATTR}],[${DORMANT_SLOT_ATTR}]`;

/** Whether `el` sits in a hidden window tab or a dormant pane-stack member. */
export function isInHiddenPaneTab(el: Element): boolean {
    return el.closest(HIDDEN_CONTAINER_SELECTOR) != null;
}

/**
 * The `data-window-tab-hidden` value for a `data-pane-overlay` element that
 * renders through a Portal: under `document.body` it has no hidden-tab
 * ancestor, so it marks itself while the window tab it was opened from isn't
 * displayed. Call in the component's scope:
 * `data-window-tab-hidden={hiddenTab()}`.
 */
export function useHiddenTabMarker(): Accessor<"" | undefined> {
    const displayed = useWindowTabDisplayed();
    return () => (displayed() ? undefined : "");
}

// Every registered overlay's re-measure, and the auto service's sweep. Hiding
// or showing a tab changes neither an overlay's size nor its own style, so no
// per-overlay observer sees it: the marker watcher below re-measures them all.
const overlayUpdaters = new Set<() => void>();
let autoOverlaySweep: (() => void) | null = null;
let hiddenTabWatcherStarted = false;

/** Set by pane-overlay-auto.ts: re-measure every auto-discovered overlay now. */
export function __setAutoOverlaySweep(sweep: () => void): void {
    autoOverlaySweep = sweep;
}

/** Re-measure every overlay now, e.g. after a tab was hidden or shown. */
export function remeasurePaneOverlays(): void {
    for (const update of [...overlayUpdaters]) update();
    autoOverlaySweep?.();
}

/** Watch the hidden-tab and dormant-slot markers once, for the whole document. */
export function startHiddenTabWatcher(): void {
    if (hiddenTabWatcherStarted || typeof MutationObserver === "undefined" || typeof document === "undefined") return;
    hiddenTabWatcherStarted = true;
    new MutationObserver(() => remeasurePaneOverlays()).observe(document.documentElement, {
        subtree: true,
        attributes: true,
        attributeFilter: [HIDDEN_TAB_ATTR, DORMANT_SLOT_ATTR],
    });
}

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

// Listeners told when an explicitly registered overlay that covered a native
// browser pane goes away (a menu closing over a page). The keyboard was left
// with the app while it was open; the focus manager gives it back.
const releaseListeners = new Set<() => void>();
// Ids (in `overlayRects`) of overlays that take the keyboard while open: menus
// and popovers, registered with `{ keys: true }`. The rest only clip (a status
// bar tooltip, a drag hole): they neither take focus nor handle Escape/arrows.
const keyOverlays = new Set<number>();
export interface PaneOverlayOptions {
    /** The overlay takes the keyboard while open (Escape closes it, arrows move in it). */
    keys?: boolean;
}
let releaseScheduled = false;

/** Run `cb` after an overlay that covered a browser pane is released. Returns an unsubscribe. */
export function onPaneOverlayReleased(cb: () => void): () => void {
    releaseListeners.add(cb);
    return () => releaseListeners.delete(cb);
}

/** Drop an explicitly registered overlay's rect; true when there was one. */
function dropOverlayRect(id: number): boolean {
    const rect = overlayRects.get(id);
    if (!overlayRects.delete(id)) return false;
    const holdsKeys = keyOverlays.has(id);
    if (rect != null && holdsKeys && releaseListeners.size > 0 && anyPaneIntersects(rect) && !releaseScheduled) {
        releaseScheduled = true;
        requestAnimationFrame(() => {
            releaseScheduled = false;
            for (const cb of [...releaseListeners]) cb();
        });
    }
    return true;
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
    // rects directly (browser_panes/clip.rs::set_pane_overlay_clip — no DPI
    // scaling on that side). A CSS-px rect therefore punches the airspace
    // hole in the wrong place and at the wrong size on any display scale
    // != 100%: black voids where the hole misses the overlay, and the pane
    // covering the overlay where the hole is too small (the "offset menus /
    // black spots / hidden" airspace bug — see
    // docs/analysis/ANALYSIS_BROWSER_PANE_AIRSPACE_ARCHITECTURE_2026_05_30.md).
    // Mirror use-pane-rect-sync.ts::paneRect's `Math.round(v * dpr)` convention
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
    // bare placeholder; use-freeze-frame.ts listens for this event to show a
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
    getApi().browserPanes.setOverlayClip(rectsToSend, window_label).catch(() => {});
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
 * `window.resize`, whenever a `ResizeObserver` reports the element's box
 * changing, and whenever its window tab or pane-stack member is hidden or
 * shown. An overlay in a hidden tab or dormant member keeps its real size and
 * position, so without that it would keep punching a hole through the panes
 * of the tab on screen. It deregisters on unmount. Overlays that move without
 * any size change (dropped anchors) may still need an `IntersectionObserver`
 * layered on.
 *
 * Safe to nest — each call registers its own rect, the union is applied.
 * No-op on platforms without native pane HWNDs (backend IPC is a no-op).
 */
export interface PaneOverlayHandle {
    /** Re-measure now (the style/size observers already cover most moves). */
    update(): void;
    /** Drop this overlay's hole. Idempotent. Must run on every close path —
     *  a leaked registration leaves a see-through, click-through hole in the
     *  pane (on macOS the whole pane ignores the mouse while any hole is open). */
    release(): void;
}

/**
 * Imperative sibling of `usePaneOverlay` for overlays that aren't Solid
 * components — `showJsContextMenu` (app/host/js-context-menu.ts) builds its menu with plain DOM
 * and so can't call a hook. Same map, same `sendClip()`, same observers.
 *
 * Registering early is fine: an element that is `display:none` (zero box) or
 * `visibility:hidden` contributes no hole, and the style observer re-measures
 * once it is placed and revealed.
 *
 * Needed because the auto-discovery service (pane-overlay-auto.ts) only runs on
 * Windows, so on macOS/Linux a `data-pane-overlay` attribute alone registers
 * nothing — see SPEC_MACOS_BROWSER_PANE_CONTEXT_MENU_2026_09_24.md.
 */
export function registerPaneOverlay(el: HTMLElement, opts?: PaneOverlayOptions): PaneOverlayHandle {
    const id = nextOverlayId++;
    if (opts?.keys) keyOverlays.add(id);
    let released = false;
    registeredOverlayEls.add(el);
    const update = (): void => {
        if (released) return;
        const rect = rectFromElement(el);
        const hidden = !el.isConnected || getComputedStyle(el).visibility === "hidden" || isInHiddenPaneTab(el);
        if (!hidden && rect.w > 0 && rect.h > 0) {
            overlayRects.set(id, rect);
        } else if (!dropOverlayRect(id)) {
            return;
        }
        sendClip();
    };
    const resizeObserver = typeof ResizeObserver !== "undefined" ? new ResizeObserver(() => update()) : undefined;
    resizeObserver?.observe(el);
    const styleObserver = typeof MutationObserver !== "undefined" ? new MutationObserver(() => update()) : undefined;
    styleObserver?.observe(el, { attributes: true, attributeFilter: ["style"] });
    window.addEventListener("resize", update);
    overlayUpdaters.add(update);
    startHiddenTabWatcher();
    update();
    return {
        update,
        release: () => {
            if (released) return;
            released = true;
            registeredOverlayEls.delete(el);
            overlayUpdaters.delete(update);
            window.removeEventListener("resize", update);
            resizeObserver?.disconnect();
            styleObserver?.disconnect();
            const dropped = dropOverlayRect(id);
            keyOverlays.delete(id);
            if (dropped) sendClip();
        },
    };
}

// Elements currently registered through registerPaneOverlay.
const registeredOverlayEls = new WeakSet<Element>();

/**
 * Whether `el` is registered to punch a hole through the browser panes it
 * overlaps: explicitly (registerPaneOverlay) or by the Windows auto-discovery
 * service. The dev-build menu guard (menu-position.ts) uses this so a menu
 * that is meant to sit over a pane isn't reported as "behind native pane".
 */
export function isRegisteredPaneOverlay(el: Element): boolean {
    return registeredOverlayEls.has(el) || autoOverlayRects.has(el);
}

/**
 * Whether a menu or popover is drawn over browser pane `blockId`: one of the
 * overlays registered with `{ keys: true }` (`usePaneOverlay` /
 * `registerPaneOverlay`) intersects the pane. The keyboard belongs to that
 * overlay (Escape to close it, arrows to move in it), not to the page under
 * it. Not the others, which only clip: a tooltip, the drag hole, a pane's own
 * loading cover (a `data-pane-overlay` element, Windows only). False
 * for a pane with no registered rect (not a native browser pane, or not placed
 * yet). `onPaneOverlayReleased` says when one of these goes away.
 */
export function overlayCoversPane(blockId: string): boolean {
    const pane = getPaneRect(blockId);
    if (pane == null || pane.w <= 0 || pane.h <= 0) return false;
    const covers = (r: OverlayRect): boolean =>
        r.w > 0 && r.h > 0 && r.x < pane.x + pane.w && r.x + r.w > pane.x && r.y < pane.y + pane.h && r.y + r.h > pane.y;
    for (const id of keyOverlays) {
        const r = overlayRects.get(id);
        if (r != null && covers(r)) return true;
    }
    return false;
}

/** Test-only: number of live imperative/hook overlay registrations with a rect. */
export function __overlayRectCount(): number {
    return overlayRects.size;
}

/** Test-only: number of auto-discovered overlays with a rect. */
export function __autoOverlayRectCount(): number {
    return autoOverlayRects.size;
}

export function usePaneOverlay(getEl: Accessor<HTMLElement | null | undefined>, opts?: PaneOverlayOptions): void {
    const id = nextOverlayId++;
    if (opts?.keys) keyOverlays.add(id);
    // The context reaches an overlay rendered through a Portal too, where the
    // DOM marker check below can't see its tab.
    const tabDisplayed = useWindowTabDisplayed();
    const update = (): void => {
        const el = getEl();
        if (!el) return;
        const rect = rectFromElement(el);
        // A collapsed box, or one in a hidden tab or a dormant pane-stack
        // member, contributes no hole — drop the entry rather than send it.
        const shown = tabDisplayed() && el.isConnected && !isInHiddenPaneTab(el);
        if (shown && rect.w > 0 && rect.h > 0) {
            overlayRects.set(id, rect);
        } else {
            dropOverlayRect(id);
        }
        sendClip();
    };
    createEffect(on(tabDisplayed, () => update(), { defer: true }));
    let observer: ResizeObserver | undefined;
    let styleObserver: MutationObserver | undefined;
    onMount(() => {
        update();
        window.addEventListener("resize", update);
        overlayUpdaters.add(update);
        startHiddenTabWatcher();
        const el = getEl();
        if (el && typeof ResizeObserver !== "undefined") {
            // An overlay's box changing size (including a collapse to 0 under
            // `display:none`) fires no `resize`, so a ResizeObserver drops or
            // restores its rect.
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
        overlayUpdaters.delete(update);
        observer?.disconnect();
        styleObserver?.disconnect();
        const dropped = dropOverlayRect(id);
        keyOverlays.delete(id);
        if (dropped) sendClip();
    });
}
