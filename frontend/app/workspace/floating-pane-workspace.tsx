// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * FloatingPaneWorkspace — minimal chromeless workspace for the floating
 * window opened by `open_floating_pane_window` (SPEC_FLOATING_PANE_TEAROFF
 * Phase 2 / issue #1077).
 *
 * The floating window's backend state is a normal workspace+tab+block
 * (created by `TearOffBlock` in the source window). The standard
 * `initApp` → `initHostNewWindow` path picks it up via `?workspaceId=`
 * and populates atoms the same as any new window. What changes here vs.
 * the docked `<Workspace />` component is *only* the rendered chrome:
 *
 *  - no `<WindowHeader>` (which carries the tab bar + action widgets)
 *  - no `<StatusBar>`
 *  - no extra title bar — the floater renders the block's standard
 *    `BlockFrame_Header` (33 CSS px, `--header-height` in
 *    `theme.scss`'s `:root`) as its sole chrome.
 *  - window drag, installed by the `onMount` below. On Windows the host runs
 *    `Win32BeginMoveTask` (manual SetCapture + GetMessage + SetWindowPos loop,
 *    zero per-move IPC, emits hover host-side). On macOS + Linux the drag is
 *    JS-driven `get/set_window_position` polling — macOS because the patched
 *    libcef isn't in dev builds, Linux because `BeginWindowDrag` →
 *    `_NET_WM_MOVERESIZE` makes the compositor swallow all input so the renderer
 *    can't emit hover/redock (see the `jsDrivenDrag` note in `onMount`). The
 *    renderer's `mousemove` listener emits `update_floating_redock_hover` and
 *    drives position updates on both; on Windows that lives in
 *    `Win32BeginMoveTask` (§3.2 of
 *    `docs/specs/SPEC_PANE_RESIZE_AND_FLOATER_DRAG_NATIVE_LOOP_2026_06_05.md`).
 *    `preventDefault` on mousedown blocks the HTML5 dragstart
 *    pragmatic-dnd would otherwise have used, suppressing a "double
 *    tear-off" regression. See
 *    `docs/analysis/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md`
 *    for why we use this approach rather than OS HTCAPTION.
 *
 * The frontend code path is otherwise identical to the docked case:
 * Block / view-model / RPC subscriptions all behave the same.
 */

import { isLinux, isMacOS, isWindows } from "@/util/platformutil";
import { isNativeWayland } from "@/app/host/window-system";
import { FLOATER_EDGE_NONE, FLOATER_RESIZE_CURSORS, floaterEdgeDirection, isFloaterBandControl } from "@/app/workspace/floater-resize";
import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { TabContent } from "@/app/tab/tabcontent";
import { atoms, getApi } from "@/store/global";
import * as MOS from "@/store/mos";
import { isFloaterHeaderMovePress } from "./floater-header-press";
import { holdRedock, redockInProgress } from "./floating-redock-guard";
import { floaterPaneBlockId, floaterPanes, redockFloaterPane } from "./floater-panes";
import { Show, createEffect, createMemo, onCleanup, onMount, type JSX } from "solid-js";

import { createRedockArming, type RedockArming } from "./redock-arming";
import { isFloatingMaximized } from "@/app/block/floating-maximize";
import { useReapplyFloatingOnTop } from "@/app/block/floating-ontop";
import "./floating-pane-workspace.scss";

/** The longest a JS-driven drag waits for a maximized floater's restore to resize the window before
 *  it reads the restored size anyway (a restore normally lands within a frame or two). */
const UNMAXIMIZE_SETTLE_TIMEOUT_MS = 300;

/**
 * Scale factor for converting a CSS-px drag delta into the host's window-
 * position coordinate space.
 *
 * On Windows the host moves the raw HWND with `SetWindowPos` in PHYSICAL
 * pixels, so a CSS-px delta must be multiplied by `devicePixelRatio`. On
 * macOS / Linux the floater is a CEF Views window positioned via
 * `set_bounds` / `bounds` in DIP (logical) pixels — and 1 CSS px == 1 DIP —
 * so the delta is applied 1:1 (multiplying by DPR would move the window
 * `devicePixelRatio`× too fast, e.g. 2× on a Retina display). Re-read per
 * use so a mid-drag monitor crossing on Windows picks up the new DPR.
 */
function posScale(): number {
    // Only Windows positions in physical px (raw-HWND SetWindowPos); macOS and
    // Linux use CEF Views DIP (1 CSS px == 1 DIP).
    return isWindows() ? window.devicePixelRatio || 1 : 1;
}

/** What the header-drag and redock handlers need from the component. */
interface FloaterEnv {
    /** The window's label, read once at mount. */
    label: string;
    windowLabel: () => string;
    tabId: typeof atoms.activeTabId;
    ws: typeof atoms.workspace;
    /** Whether the floater is dragged by JS polling (macOS, Linux) instead of the host's move loop (Windows). */
    jsDrivenDrag: boolean;
}

type RedockGhost = { block_id?: string; dir?: number };

/** The state of one floater's drag, shared by the handlers that make up the gesture. */
interface FloaterDragState {
    dragging: boolean;
    /** On Windows: coords saved here by onMouseUp; consumed by the window_drag_ended handler which carries host
     *  cursor_x/cursor_y (eliminating the async ordering race with DOM mouseup). On non-Windows: tryRedockAtCursor
     *  is called directly from onMouseUp when hasMoved=true, so this is only a fallback. */
    pendingRedockCoords: { x: number; y: number } | null;
    /** Non-Windows only: set by onMouseMove when dragging. Gates tryRedockAtCursor in onMouseUp so a plain header
     *  click (no pixel motion) doesn't false-redock when the floater overlaps another window. */
    hasMoved: boolean;
    /** JS-driven drag state (macOS + Linux): get/set_window_position polling instead of the host native move loop. */
    jsDragClickScreenX: number;
    jsDragClickScreenY: number;
    jsDragInitWinX: number;
    jsDragInitWinY: number;
    jsDragLatestScreenX: number;
    jsDragLatestScreenY: number;
    jsDragSetPosInFlight: boolean;
    jsDragPendingPos: { x: number; y: number } | null;
    jsDragMouseDownId: number;
    /** JS-driven drag of a MAXIMIZED floater (macOS, Linux on X11): set at the press, consumed by the first move
     *  past the threshold, which restores the window to its pre-maximize size under the cursor before moving it
     *  (beginUnmaximizeJsDrag). `restoringForDrag` holds the move updates while the restore settles.
     *  SPEC_FLOATING_PANE_WINDOW_CONTROLS_REFINEMENTS_2026_10_16 §4.4. */
    unmaximizeOnMove: { fracX: number; offsetY: number } | null;
    restoringForDrag: boolean;
    /** Native Wayland (Linux): a header press waiting to become a compositor move. No client can place its own
     *  window there, so the JS-driven set_window_position does nothing; once the press moves past
     *  COMPOSITOR_MOVE_THRESHOLD_PX the compositor takes the move (start_window_drag → xdg_toplevel.move), as for
     *  the main window (useWindowDrag.linux.ts); a plain click still reaches the grip. */
    compositorPress: { x: number; y: number } | null;
    /** Windows: a press on a MAXIMIZED floater's header, waiting to move past the threshold before the host's move
     *  loop (and its restore) starts. */
    win32MaximizedPress: { x: number; y: number } | null;
    /** Per-drag session counter: incremented on every drag start and on cancel. Guards the
     *  update_floating_redock_hover IPC .then() against stale responses from a prior drag session arriving during
     *  a new drag (or after a cancel). */
    dragSessionId: number;
    /** Dwell + velocity gate: prevents accidental redock when the cursor transits over another window at speed,
     *  and — since SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md — when the user is merely repositioning the floater
     *  above the main window rather than asking to dock it. Both platform paths feed the SAME state machine
     *  (redock-arming.ts) so they cannot drift. */
    arming: RedockArming;
    /** Captured by whichever of DOM mouseup / window_drag_ended observes the release first (their relative order is
     *  not guaranteed on Windows — separate CEF IPC channels). The other reads this instead of re-deriving, and
     *  `consumeArmedAtRelease` nulls it so the second arrival cannot fire a redock on the already-redocked block. */
    armedAtRelease: boolean | null;
    /** Assigned by the non-Windows hover tracking (Windows gets its heartbeat from the host's drag tick instead, so
     *  these stay no-ops there). */
    startHoverHeartbeat: () => void;
    stopHoverHeartbeat: () => void;
    /** Sentinel for the listenEvent .then() race: if the component unmounts before the Promise resolves, the
     *  .then() immediately calls unlisten. */
    cleaned: boolean;
    /** Phase 4b — ghost pre-captured in onMouseUp before clear_floating_redock_hover broadcasts the hover-state null
     *  event (which triggers clearPlaceholder on the target renderer and would wipe the ghost before
     *  tryRedockAtCursorInner reads it). */
    capturedGhostForDrop: Promise<RedockGhost> | null;
    capturedGhostForWindow: string | null;
}

function createDragState(): FloaterDragState {
    return {
        dragging: false,
        pendingRedockCoords: null,
        hasMoved: false,
        jsDragClickScreenX: 0,
        jsDragClickScreenY: 0,
        jsDragInitWinX: 0,
        jsDragInitWinY: 0,
        jsDragLatestScreenX: 0,
        jsDragLatestScreenY: 0,
        jsDragSetPosInFlight: false,
        jsDragPendingPos: null,
        jsDragMouseDownId: 0,
        unmaximizeOnMove: null,
        restoringForDrag: false,
        compositorPress: null,
        win32MaximizedPress: null,
        dragSessionId: 0,
        arming: createRedockArming(),
        armedAtRelease: null,
        startHoverHeartbeat: () => {},
        stopHoverHeartbeat: () => {},
        cleaned: false,
        capturedGhostForDrop: null,
        capturedGhostForWindow: null,
    };
}

const COMPOSITOR_MOVE_THRESHOLD_PX = 4;

const floaterMaximized = (env: FloaterEnv): boolean => {
    const tid = env.tabId();
    const blockId = tid ? floaterPaneBlockId(tid) : undefined;
    return !!blockId && isFloatingMaximized(MOS.getObjectValue<Block>(MOS.makeORef("block", blockId))?.meta);
};

const consumeArmedAtRelease = (s: FloaterDragState): boolean => {
    const armed = s.armedAtRelease ?? s.arming.isArmed();
    s.armedAtRelease = null;
    s.arming.reset();
    return armed;
};


// ── Edge-resize (floater only) ──────────────────────────────────
// ── Edge-resize (floater only) ──────────────────────────────────
// The floater is a frameless WS_POPUP; the embedded CEF child(ren)
// consume WM_NCHITTEST, so native edge-resize via the parent wndproc
// never fires (cef-rs 146 — confirmed: a child-wndproc HTTRANSPARENT
// forwarder gets 0 hits, and WM_SYSCOMMAND(SC_SIZE) posts fine but its
// modal loop can't read the mouse because Chromium holds the OS capture
// from the DOM pointerdown). So we drive the resize from the DOM: on an
// edge pointerdown we take pointer capture (keeps receiving moves even
// as the cursor leaves the window), capture the start rect, and on each
// move SetWindowPos the new rect. Same mechanism as the JS-driven
// header MOVE below. SPEC_FLOATING_PANE_EDGE_RESIZE.

const EDGE_RESIZE_MIN_W = 240;
const EDGE_RESIZE_MIN_H = 140;

// BOTTOM, BOTTOM-LEFT, BOTTOM-RIGHT (WMSZ_* ordering). The band's
// depths, its corners and the cursor for each direction are in
// floater-resize.ts.
const edgeDirection = (e: PointerEvent): number =>
    isFloaterBandControl(e.target)
        ? FLOATER_EDGE_NONE
        : floaterEdgeDirection(e.clientX, e.clientY, window.innerWidth, window.innerHeight);
// Show the resize cursor for a direction (0 clears it) on <html>. The band
// is invisible, so the cursor is the only sign it is there, and a cursor
// set on <body> loses to any child that sets its own (a header button).
// floating-pane-workspace.scss applies it to every element.
const showEdgeCursor = (d: number): void => {
    const root = document.documentElement;
    if (d) root.dataset.floaterEdge = FLOATER_RESIZE_CURSORS[d];
    else delete root.dataset.floaterEdge;
};

// Coalesce moves the same way the header MOVE does: one IPC in
// flight at a time, last position wins (no timers/RAF).
function createRectSender(label: string): (r: { x: number; y: number; width: number; height: number }) => void {
    let rectInFlight = false;
    let pendingRect: { x: number; y: number; width: number; height: number } | null = null;
    const sendRect = (r: { x: number; y: number; width: number; height: number }): void => {
        if (rectInFlight) { pendingRect = r; return; }
        rectInFlight = true;
        getApi().windows.setRect(label, r)
            .catch(() => {})
            .finally(() => {
                rectInFlight = false;
                if (pendingRect) { const p = pendingRect; pendingRect = null; sendRect(p); }
            });
    };
    return sendRect;
}

function installEdgeResize(label: string): void {
    let resizing = false;
    let dir = 0;
    let startX = 0; // screen px at pointerdown
    let startY = 0;
    let startRect = { x: 0, y: 0, w: 0, h: 0 }; // physical px (Windows) / DIP (macOS, Linux)
    const sendRect = createRectSender(label);

    const onPointerDown = (e: PointerEvent): void => {
        if (e.button !== 0) return;
        const d = edgeDirection(e);
        if (d === 0) return;
        // Win the event over the header-drag + content handlers, and
        // stop text selection.
        e.preventDefault();
        e.stopImmediatePropagation();
        dir = d;
        // The cursor stays this one for the whole drag, wherever the pointer goes.
        showEdgeCursor(d);
        startX = e.screenX;
        startY = e.screenY;
        const target = e.target as Element;
        // Take capture synchronously so moves keep flowing even as the
        // cursor leaves the window; then fetch the start rect.
        try { target.setPointerCapture(e.pointerId); } catch { /* ignore */ }
        getApi().windows.getRect(label).then((r) => {
            if (r.width > 0 && r.height > 0) {
                startRect = { x: r.x, y: r.y, w: r.width, h: r.height };
                resizing = true;
            } else {
                // IPC returned zeros (timeout or lookup failure) — abort.
                try { target.releasePointerCapture(e.pointerId); } catch { /* ignore */ }
            }
        }).catch(() => {
            try { target.releasePointerCapture(e.pointerId); } catch { /* ignore */ }
        });
    };

    const onPointerMove = (e: PointerEvent): void => {
        if (!resizing) {
            // Hover feedback only.
            showEdgeCursor(edgeDirection(e));
            return;
        }
        // screenX/Y are CSS px. Windows: rect is physical px → scale by DPR.
        // macOS / Linux: rect is DIP (1 CSS px == 1 DIP) → scale 1:1.
        // posScale() encodes this: DPR on Windows, 1 elsewhere (mirrors drag).
        const scale = posScale();
        const dx = Math.round((e.screenX - startX) * scale);
        const dy = Math.round((e.screenY - startY) * scale);
        let { x, y, w, h } = startRect;
        const left = dir === 1 || dir === 4 || dir === 7;
        const right = dir === 2 || dir === 5 || dir === 8;
        const top = dir === 3 || dir === 4 || dir === 5;
        const bottom = dir === 6 || dir === 7 || dir === 8;
        if (left) { x += dx; w -= dx; }
        if (right) { w += dx; }
        if (top) { y += dy; h -= dy; }
        if (bottom) { h += dy; }
        // Clamp to a min size; when dragging a top/left edge, pin the
        // opposite edge by not letting the origin run past the min.
        if (w < EDGE_RESIZE_MIN_W) { if (left) x -= EDGE_RESIZE_MIN_W - w; w = EDGE_RESIZE_MIN_W; }
        if (h < EDGE_RESIZE_MIN_H) { if (top) y -= EDGE_RESIZE_MIN_H - h; h = EDGE_RESIZE_MIN_H; }
        sendRect({ x, y, width: w, height: h });
    };

    const onPointerUp = (e: PointerEvent): void => {
        if (!resizing) return;
        resizing = false;
        try { (e.target as Element).releasePointerCapture(e.pointerId); } catch { /* ignore */ }
        showEdgeCursor(0);
    };

    // Capture phase + registered before the header listener so the
    // edge wins (stopImmediatePropagation halts the header handler).
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("pointermove", onPointerMove, true);
    document.addEventListener("pointerup", onPointerUp, true);
    onCleanup(() => {
        document.removeEventListener("pointerdown", onPointerDown, true);
        document.removeEventListener("pointermove", onPointerMove, true);
        document.removeEventListener("pointerup", onPointerUp, true);
        showEdgeCursor(0);
    });
}

// ── Header drag ─────────────────────────────────────────────────

// dragId guards the drain: if a new mousedown starts (incrementing
// jsDragMouseDownId) while a set_window_position is in-flight, the old
// drain's .finally discards jsDragPendingPos instead of applying a delta
// from the previous drag's origin to the new drag's jsDragInitWinX/Y.
function jsDragSendPos(s: FloaterDragState, env: FloaterEnv, dragId: number, x: number, y: number): void {
    if (s.jsDragSetPosInFlight) {
        s.jsDragPendingPos = { x, y };
        return;
    }
    s.jsDragSetPosInFlight = true;
    getApi().windows.setPosition(env.label, x, y)
        .catch(() => {})
        .finally(() => {
            s.jsDragSetPosInFlight = false;
            if (s.jsDragPendingPos && dragId === s.jsDragMouseDownId) {
                const { x: nx, y: ny } = s.jsDragPendingPos;
                s.jsDragPendingPos = null;
                jsDragSendPos(s, env, dragId, nx, ny);
            } else {
                s.jsDragPendingPos = null;
            }
        });
}

// Restore a maximized floater at the start of a JS-driven drag: the host
// toggle restores it natively (CEF Views on macOS and Linux), then the
// window is placed so the cursor holds the same part of the header, and
// the drag continues from there.
function beginUnmaximizeJsDrag(s: FloaterDragState, env: FloaterEnv, dragId: number): void {
    const grab = s.unmaximizeOnMove;
    if (!grab) return;
    s.unmaximizeOnMove = null;
    const tid = env.tabId();
    const blockId = tid ? floaterPaneBlockId(tid) : undefined;
    if (!blockId) return; // no block yet: the drag moves it at its current size
    s.restoringForDrag = true;
    const settled = new Promise<void>((resolve) => {
        const done = () => {
            window.removeEventListener("resize", done);
            clearTimeout(timer);
            resolve();
        };
        const timer = setTimeout(done, UNMAXIMIZE_SETTLE_TIMEOUT_MS);
        window.addEventListener("resize", done);
    });
    getApi()
        .windows.toggleFloatingMaximize(env.label, blockId)
        .then(() => settled)
        .then(() => getApi().windows.getRect(env.label))
        .then((rect) => {
            if (dragId !== s.jsDragMouseDownId) return; // released meanwhile: it stays restored where it is
            const x = Math.round(s.jsDragLatestScreenX - grab.fracX * rect.width);
            const y = Math.round(s.jsDragLatestScreenY - grab.offsetY);
            s.jsDragInitWinX = x;
            s.jsDragInitWinY = y;
            s.jsDragClickScreenX = s.jsDragLatestScreenX;
            s.jsDragClickScreenY = s.jsDragLatestScreenY;
            jsDragSendPos(s, env, dragId, x, y);
        })
        .catch((err) => console.error("[floating-pane] restore for drag failed", err))
        .finally(() => {
            s.restoringForDrag = false;
        });
}

/** The press of a JS-driven drag (macOS + Linux). */
function startJsDrag(s: FloaterDragState, env: FloaterEnv, e: MouseEvent): void {
    // JS-driven drag (macOS + Linux). macOS: BeginWindowDrag needs a
    // patched libcef absent in dev builds. Linux: BeginWindowDrag →
    // _NET_WM_MOVERESIZE makes the compositor swallow all input, so the
    // renderer goes dark (no mousemove → no hover ghosts; no mouseup
    // gate → no redock). Both use get/set_window_position polling.
    s.jsDragMouseDownId += 1;
    s.dragSessionId += 1;
    const myId = s.jsDragMouseDownId;
    s.jsDragClickScreenX = e.screenX;
    s.jsDragClickScreenY = e.screenY;
    s.jsDragLatestScreenX = e.screenX;
    s.jsDragLatestScreenY = e.screenY;
    s.hasMoved = false;
    s.pendingRedockCoords = null;
    s.arming.reset();
    s.armedAtRelease = null;
    s.capturedGhostForDrop = null;
    s.capturedGhostForWindow = null;
    s.unmaximizeOnMove = null;
    s.restoringForDrag = false;
    // The grab point, as a fraction of the header's width and a
    // fixed offset from the top: where the cursor holds the
    // restored window (the main window's rule, window_snap.rs
    // `unmaximize_drag_origin`).
    const grab = { fracX: e.clientX / Math.max(1, window.innerWidth), offsetY: e.clientY };
    Promise.all([
        getApi().windows.getPosition(env.label),
        getApi().windows.isMaximized(env.label).catch(() => null),
    ])
        .then(([pos, answer]) => {
            // The host can't tell (null): go by the stored placement.
            const maximized = answer ?? floaterMaximized(env);
            if (myId !== s.jsDragMouseDownId) return;
            s.jsDragInitWinX = pos.x;
            s.jsDragInitWinY = pos.y;
            if (maximized) {
                // Restore on the first real move, not on a plain click.
                s.unmaximizeOnMove = grab;
                s.dragging = true;
                s.startHoverHeartbeat();
                if (s.jsDragLatestScreenX !== s.jsDragClickScreenX || s.jsDragLatestScreenY !== s.jsDragClickScreenY) {
                    s.hasMoved = true;
                    beginUnmaximizeJsDrag(s, env, myId);
                }
                return;
            }
            const movedDuringIPC =
                s.jsDragLatestScreenX !== s.jsDragClickScreenX ||
                s.jsDragLatestScreenY !== s.jsDragClickScreenY;
            if (movedDuringIPC) {
                // Motion happened before dragging was armed; set
                // hasMoved so onMouseUp's gate allows tryRedockAtCursor.
                s.hasMoved = true;
                const scale = posScale();
                jsDragSendPos(s, env,
                    myId,
                    s.jsDragInitWinX + Math.round((s.jsDragLatestScreenX - s.jsDragClickScreenX) * scale),
                    s.jsDragInitWinY + Math.round((s.jsDragLatestScreenY - s.jsDragClickScreenY) * scale),
                );
            }
            s.dragging = true;
            // Started here, not in mousedown: a plain header click
            // can release before this IPC resolves, and onMouseUp
            // returns early while `dragging` is still false — so a
            // heartbeat armed up front would never be cleared and
            // would outlive the gesture. reagent P1 on #3124.
            s.startHoverHeartbeat();
        })
        .catch(() => {});
}

// Win32BeginMoveTask manual loop (zero per-move IPC, no DPR math).
// The host owns motion + capture and emits update_floating_redock_hover
// itself while the renderer is dark; renderer sees mouseup via the
// dispatched WM_LBUTTONUP balance (PR #1181 §5.1).
function startWin32Drag(s: FloaterDragState, env: FloaterEnv): void {
    getApi().windows.startDrag(env.label).catch(() => {});
    s.dragging = true;
    s.hasMoved = false;
    s.pendingRedockCoords = null;
    s.arming.reset();
    s.armedAtRelease = null;
    s.capturedGhostForDrop = null;
    s.capturedGhostForWindow = null;
    s.dragSessionId += 1;
}

function onHeaderMouseDown(s: FloaterDragState, env: FloaterEnv, e: MouseEvent): void {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement | null;
    if (!target) return;
    // A control in the header (a button, a pane tab pill) keeps its own press; the pane grip
    // moves the floater like the rest of the header.
    if (!isFloaterHeaderMovePress(target)) return;

    // Load-bearing: blocks pragmatic-dnd's HTML5 dragstart, preventing
    // the double-tear-off regression (ANALYSIS_FLOATING_PANE_HEADER_DRAG
    // §"Tear-off conflict").
    e.preventDefault();

    if (isLinux() && isNativeWayland()) {
        s.compositorPress = { x: e.clientX, y: e.clientY };
        return;
    }


    if (env.jsDrivenDrag) {
        startJsDrag(s, env, e);
        return;
    }

    // Windows only now (macOS + Linux returned via jsDrivenDrag above).
    // A MAXIMIZED floater's press waits for real movement, as native
    // Wayland's does: the host restores a maximized floater as its move
    // loop starts, so starting it on the press would restore on a plain
    // click, and a double-click would restore and then re-maximize.
    // SPEC_FLOATING_PANE_WINDOW_CONTROLS_REFINEMENTS_2026_10_16 §3.3.
    if (floaterMaximized(env)) {
        s.win32MaximizedPress = { x: e.clientX, y: e.clientY };
        return;
    }
    startWin32Drag(s, env);
}

function onHeaderMouseUp(s: FloaterDragState, env: FloaterEnv, e: MouseEvent): void {
    s.compositorPress = null;
    s.win32MaximizedPress = null;
    s.unmaximizeOnMove = null;
    // Invalidate any in-flight JS-driven get_window_position IPC so a race
    // where mouseup fires before the promise resolves doesn't arm dragging.
    if (env.jsDrivenDrag) s.jsDragMouseDownId += 1;
    // Before the !dragging early return: a click that never became a
    // drag must still tear down anything mousedown started.
    s.stopHoverHeartbeat();
    if (!s.dragging) return;
    s.dragging = false;
    // On macOS/Linux, paneRect() returns unchanged client coords after a
    // window move, so browser-view's syncPosition dedupe guard skips the
    // browser_pane_resize re-send. Signal it to force one so the
    // NativeWidgetMacNSWindow overlay repositions to the new window frame.
    if (env.jsDrivenDrag) {
        window.dispatchEvent(
            new CustomEvent("floating-pane-js-drag-ended", { detail: { label: env.label } }),
        );
    }
    // Phase 4b — dispatch get_floating_redock_target BEFORE clear_floating_redock_hover.
    // Both are fire-and-forget IPCs from the same floater renderer → CEF backend
    // channel (FIFO). The ghost read queues ahead of the event broadcast that
    // triggers clearPlaceholder on the target renderer. This avoids the cross-
    // process race where the target's set_floating_redock_target(null) arrives
    // before tryRedockAtCursorInner's delayed call to get_floating_redock_target.
    //
    // Guarded, and NOT idempotent without the guard: on Windows
    // window_drag_ended can run first, and it captures the ghost and
    // then calls consumeArmedAtRelease(), which resets arming. Re-reading
    // arming.target() here would overwrite that good capture with null
    // and leave the in-flight redock to fall back to an empty ghost —
    // docking the pane somewhere other than where the user saw it, with
    // no error. The pre-PR code got away with an unguarded capture only
    // because it read dwellCurrentHoverTarget, which nothing reset.
    // reagent P1 on #3124.
    if (!s.capturedGhostForDrop) {
        s.capturedGhostForWindow = s.arming.target();
        s.capturedGhostForDrop = s.capturedGhostForWindow
            ? getApi().windows.getFloatingRedockTarget(s.capturedGhostForWindow).catch(() => ({}))
            : Promise.resolve({});
    }
    const preGhostWindow = s.capturedGhostForWindow;
    getApi().windows.clearFloatingRedockHover().catch(() => {});
    if (isWindows()) {
        // On Windows the redock itself is committed by window_drag_ended
        // (it carries host cursor coords, so it doesn't have to trust the
        // DOM's). The two events travel separate CEF IPC channels and
        // their relative order is not guaranteed, so record the arm
        // decision here and let whichever handler runs second read it
        // rather than re-deriving from state the first one may have
        // already torn down.
        s.armedAtRelease = s.arming.isArmed();
        s.pendingRedockCoords = { x: e.screenX, y: e.screenY };
    } else if (s.hasMoved) {
        // Non-Windows (macOS + Linux): the JS-driven path keeps the
        // renderer receiving mousemove+mouseup normally, so the redock
        // commits here. Consuming resets the arming state, which is what
        // stops a late window_drag_ended (the Linux race) from firing a
        // second tryRedockAtCursor on the already-redocked block.
        //
        // Invalidate in-flight IPC .then() as well, so a stale hover
        // response cannot re-arm after this release has been consumed
        // (reagent P1 on #1249's follow-up).
        s.dragSessionId += 1;
        if (consumeArmedAtRelease(s)) {
            void tryRedockAtCursor(s, env, e.screenX, e.screenY, preGhostWindow);
        }
    }
}

// Helper: safe unlisten registration using the `cleaned` sentinel.
// If the component unmounts before listenEvent's Promise resolves,
// the .then() calls unlisten immediately rather than storing into a
// stale closure — prevents accumulated listener leaks across tear-off
// / remount cycles.
function safeListenEvent<T>(s: FloaterDragState, event: string, handler: (payload: T) => void): () => void {
    let unlisten: (() => void) | null = null;
    getApi().listen<T>(event, handler).then(u => {
        if (s.cleaned) { u(); } else { unlisten = u; }
    }).catch(() => {});
    return () => { unlisten?.(); };
}

// Esc-cancel from host — clear pending redock coords to prevent a
// spurious dock attempt on the next mouseup.
function listenDragCancelled(s: FloaterDragState, env: FloaterEnv): () => void {
    return safeListenEvent<{ label: string }>(
        s,
        "window_drag_cancelled",
        (ev) => {
            if (!ev.label || ev.label === env.label) {
                if (env.jsDrivenDrag) s.jsDragMouseDownId += 1;
                s.dragSessionId += 1;
                s.dragging = false;
                s.stopHoverHeartbeat();
                s.hasMoved = false;
                s.pendingRedockCoords = null;
                s.arming.reset();
                s.armedAtRelease = null;
                s.capturedGhostForDrop = null;
                s.capturedGhostForWindow = null;
            }
        },
    );
}

// Host signals drag loop ended. Always resets dragging and clears hover.
// On Windows: cursor_x/cursor_y (physical px) are included so the handler
// is self-contained — eliminates the ordering race where DOM mouseup and
// this event arrive via separate CEF IPC channels (relative order not
// guaranteed). On non-Windows: falls back to pendingRedockCoords set by
// onMouseUp if the OS delivered a DOM mouseup (BeginWindowDrag case).
function listenDragEnded(s: FloaterDragState, env: FloaterEnv): () => void {
    return safeListenEvent<{
        label: string;
        moved: boolean;
        cursor_x?: number;
        cursor_y?: number;
    }>(
        s,
        "window_drag_ended",
        (ev) => {
            if (!ev.label || ev.label !== env.label) return;
            s.dragging = false;
            s.stopHoverHeartbeat();
            // Phase 4b — pre-capture ghost if onMouseUp hasn't already done so
            // (window_drag_ended can arrive before DOM mouseup on Windows — separate
            // CEF IPC channels). Guards against double-consume:
            // get_floating_redock_target removes the entry atomically, so the
            // second caller would get {} anyway, but skip the IPC entirely.
            //
            // MUST run before consumeArmedAtRelease() below, which resets the
            // arming state and with it arming.target().
            if (!s.capturedGhostForDrop) {
                const preGhostWindow = s.arming.target();
                s.capturedGhostForWindow = preGhostWindow;
                s.capturedGhostForDrop = preGhostWindow
                    ? getApi().windows.getFloatingRedockTarget(preGhostWindow).catch(() => ({}))
                    : Promise.resolve({});
            }
            // Whichever of DOM mouseup / this event ran first recorded the
            // arm decision; this consumes it (and resets arming) so the two
            // orderings agree and neither can redock twice.
            const armedAtEnd = consumeArmedAtRelease(s);
            // Always clear hover — safety net for non-Windows where onMouseUp
            // may not have fired (BeginWindowDrag absorbs the release).
            getApi().windows.clearFloatingRedockHover().catch(() => {});
            if (ev.moved && !s.cleaned && armedAtEnd) {
                // Prefer host-provided cursor coords (physical px → CSS px via
                // posScale). On Windows these are always present. On non-Windows
                // fall back to pendingRedockCoords saved by onMouseUp if the OS
                // delivered a DOM mouseup; otherwise redock is F3-pending.
                const coords = (() => {
                    if (typeof ev.cursor_x === "number" && typeof ev.cursor_y === "number") {
                        const scale = posScale();
                        return { x: ev.cursor_x / scale, y: ev.cursor_y / scale };
                    }
                    return s.pendingRedockCoords;
                })();
                s.pendingRedockCoords = null;
                if (coords) void tryRedockAtCursor(s, env, coords.x, coords.y, s.capturedGhostForWindow);
            } else {
                s.pendingRedockCoords = null;
            }
        },
    );
}

// On Windows, Win32BeginMoveTask emits `floating-redock:hover-state` at
// 50 ms with cursor_x/cursor_y (physical px). Both gates apply:
// 1. Velocity: compute CSS-px/s from successive event positions; if
//    > REDOCK_VELOCITY_PX_PER_S reset dwell clock and disarm.
// 2. Dwell: arm only after the same target has been seen continuously for
//    REDOCK_DWELL_MS ms (checked after the velocity gate passes).
function listenHoverState(s: FloaterDragState): () => void {
    if (!isWindows()) return () => {};
    return safeListenEvent<{
        target_label?: string | null;
        cursor_x?: number;
        cursor_y?: number;
    }>(
        s,
        "floating-redock:hover-state",
        (ev) => {
            if (!s.dragging) return;
            // cursor_x/cursor_y are physical px on Windows — divide by
            // posScale() to get the CSS px the arming module expects
            // (1 DIP on non-HiDPI, 0.5 CSS on 2×). The host emits these
            // both from WM_MOUSEMOVE and, since the dwell spec, from the
            // 100ms drag tick — so samples keep arriving while the
            // cursor is stationary and dwell is measured, not inferred.
            const scale = posScale();
            const { clearIndicator } = s.arming.sample({
                target: ev.target_label ?? null,
                // Absent on the target-only teardown broadcast; the
                // module reuses its last position rather than reading
                // the gap as a jump from the origin.
                x: typeof ev.cursor_x === "number" ? ev.cursor_x / scale : undefined,
                y: typeof ev.cursor_y === "number" ? ev.cursor_y / scale : undefined,
                t: performance.now(),
            });
            if (clearIndicator) {
                getApi().windows.clearFloatingRedockHover().catch(() => {});
            }
        },
    );
}

// macOS + Linux use the JS-driven drag (jsDrivenDrag), so the renderer
// owns the gesture and keeps receiving mousemove. This handler drives
// both the window position (jsDragSendPos) and the redock hover emit.
// On Windows the renderer's mousemove is dark during the drag —
// Win32BeginMoveTask emits hover host-side instead (§3.2 / §3.3 of spec).
function createHoverEmitter(s: FloaterDragState, env: FloaterEnv): (screenX: number, screenY: number) => void {
    const HOVER_THROTTLE_MS = 50;
    let lastHoverAt = 0;
    // Guards against an out-of-order IPC response feeding the arming
    // module a sample older than one it has already applied, which
    // would drag the dwell clock backwards.
    let lastAppliedSampleT = 0;

    // Resolve the target under the cursor and feed the result to the
    // arming module. The hover IPC is no longer pre-gated on a local
    // dwell estimate: the module owns the arming decision now, and it
    // needs a continuous sample stream to make it. The ghost stays
    // hidden until armed, so emitting early costs nothing visible.
    const emitHover = (screenX: number, screenY: number) => {
        const now = performance.now();
        if (now - lastHoverAt < HOVER_THROTTLE_MS) return;
        lastHoverAt = now;
        const sourceLabel = env.windowLabel();
        if (!sourceLabel) return;
        const scale = posScale();
        const capturedSessionId = s.dragSessionId;
        // Sampled at REQUEST time so the velocity the module sees is the
        // cursor's, not the IPC round-trip's.
        const sampleX = screenX;
        const sampleY = screenY;
        const sampleT = now;
        getApi().windows.updateFloatingRedockHover({
            source_label: sourceLabel,
            x: Math.round(screenX * scale),
            y: Math.round(screenY * scale),
        }).then((res) => {
            if (capturedSessionId !== s.dragSessionId) return;
            if (sampleT < lastAppliedSampleT) return;
            lastAppliedSampleT = sampleT;
            const { clearIndicator } = s.arming.sample({
                target: res?.target_label ?? null,
                x: sampleX,
                y: sampleY,
                t: sampleT,
            });
            if (clearIndicator) {
                getApi().windows.clearFloatingRedockHover().catch(() => {});
            }
        }).catch(() => {});
    };
    return emitHover;
}

    // The renderer stops receiving mousemove the instant the cursor
    // holds still, so dwell fed from mousemove alone cannot advance
    // during exactly the gesture it measures. This is the JS
    // counterpart of the host's DRAG_TICK heartbeat on Windows
    // (crates/cef/src/ui_tasks/drag.rs) — same cadence, same reason.
    // SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md §5.1.
function installHoverHeartbeat(s: FloaterDragState, emitHover: (screenX: number, screenY: number) => void): void {
    const HOVER_HEARTBEAT_MS = 100;
    let heartbeatTimer: ReturnType<typeof setInterval> | null = null;
    s.startHoverHeartbeat = () => {
        if (heartbeatTimer !== null) return;
        heartbeatTimer = setInterval(() => {
            // hasMoved gates the redock in onMouseUp, so emitting before the
            // drag has moved would paint a ghost promising a dock that
            // cannot happen — a press-and-hold on the header would arm
            // the indicator after the dwell and then do nothing on
            // release (codex P2 on #3124). The host heartbeat is gated
            // on its own `moves > 0` for the same reason.
            if (!s.dragging || !s.hasMoved) return;
            emitHover(s.jsDragLatestScreenX, s.jsDragLatestScreenY);
        }, HOVER_HEARTBEAT_MS);
    };
    s.stopHoverHeartbeat = () => {
        if (heartbeatTimer !== null) {
            clearInterval(heartbeatTimer);
            heartbeatTimer = null;
        }
    };
}

function installMouseMoveTracking(s: FloaterDragState, env: FloaterEnv): () => void {
    if (isWindows()) return () => {};

    const emitHover = createHoverEmitter(s, env);
    installHoverHeartbeat(s, emitHover);

    const onMouseMove = (e: MouseEvent) => {
        if (env.jsDrivenDrag) {
            // Track latest coords even before dragging is armed — needed for
            // catch-up in onMouseDown's get_window_position .then() callback.
            s.jsDragLatestScreenX = e.screenX;
            s.jsDragLatestScreenY = e.screenY;
        }
        if (!s.dragging) return;
        s.hasMoved = true;
        if (env.jsDrivenDrag && s.unmaximizeOnMove) beginUnmaximizeJsDrag(s, env, s.jsDragMouseDownId);
        if (env.jsDrivenDrag && !s.restoringForDrag) {
            // JS-driven position update — compute delta from mousedown origin
            // and call set_window_position with a one-in-flight + coalesce guard.
            const scale = posScale();
            jsDragSendPos(s, env,
                s.jsDragMouseDownId,
                s.jsDragInitWinX + Math.round((e.screenX - s.jsDragClickScreenX) * scale),
                s.jsDragInitWinY + Math.round((e.screenY - s.jsDragClickScreenY) * scale),
            );
        }
        emitHover(e.screenX, e.screenY);
    };
    document.addEventListener("mousemove", onMouseMove);
    return () => document.removeEventListener("mousemove", onMouseMove);
}

// ── Redock ──────────────────────────────────────────────────────

async function tryRedockAtCursor(s: FloaterDragState, env: FloaterEnv, screenX: number, screenY: number, expectedTarget: string | null) {
    await holdRedock(() => tryRedockAtCursorInner(s, env, screenX, screenY, expectedTarget));
}

/** The window under the cursor that a redock would dock into, or null when there is none to dock into. */
async function resolveRedockWindow(
    s: FloaterDragState,
    ourLabel: string,
    px: number,
    py: number,
    expectedTarget: string | null
): Promise<{ label: string; window_id: string } | null> {
    let target: { label: string | null; window_id: string | null };
    try {
        // exclude_label = our own floater's label. Without this,
        // the host's Z-order walk would return the floater itself
        // — it's at the cursor (the JS-driven drag follows the
        // cursor), so it's always topmost where the cursor is.
        target = await getApi().windows.resolveWindowAtCursor({
            x: px,
            y: py,
            exclude_label: ourLabel,
        });
    } catch (e) {
        console.error("[floating-pane] resolve_window_at_cursor failed", e);
        return null;
    }
    if (s.cleaned) return null;
    if (!target.label || !target.window_id) {
        // Cursor over desktop, external app, or our own floater
        // — leave floater at the dropped position.
        return null;
    }
    // The arming decision was made about a specific window, but this
    // resolve runs independently at release time. If the cursor left
    // that window after the arming sample and before the release —
    // within one heartbeat, so no sample caught it — the two disagree,
    // and docking here would drop the pane into a window that never
    // showed a ghost. Trust the armed target, not the late resolve.
    // codex P1 on #3124.
    if (expectedTarget && target.label !== expectedTarget) {
        console.log(
            "[floating-pane] redock aborted: cursor left the armed target",
            { expectedTarget, resolved: target.label },
        );
        return null;
    }
    return { label: target.label, window_id: target.window_id };
}

/** The workspace of the window a floater is docking into. */
async function resolveTargetWorkspace(s: FloaterDragState, windowId: string): Promise<Workspace | null> {
    // Resolve target's active tab via the MuxObj graph:
    // window → workspace.activetabid. Use the non-pinning
    // `reloadMuxObject` — `loadAndPinMuxObject` would bump
    // `refCount` with no matching unpin in this async flow,
    // leaking one ref on each of the target Window + Workspace
    // per successful redock so the cache cleanup never evicts
    // them.
    let targetWs: Workspace;
    try {
        const targetWindow = await MOS.reloadMuxObject<MuxWindow>(
            MOS.makeORef("window", windowId),
        );
        targetWs = await MOS.reloadMuxObject<Workspace>(
            MOS.makeORef("workspace", targetWindow.workspaceid),
        );
    } catch (e) {
        console.error(
            "[floating-pane] failed to resolve target window's workspace",
            e,
        );
        return null;
    }
    if (s.cleaned) return null;
    return targetWs;
}

async function tryRedockAtCursorInner(s: FloaterDragState, env: FloaterEnv, screenX: number, screenY: number, expectedTarget: string | null) {
    const ourLabel = env.windowLabel();
    if (!ourLabel || s.cleaned) return;
    // `mouseup.screenX/Y` are CSS px. Host coordinate space: physical px
    // on Windows (× DPR), DIP on macOS/Linux (× 1) — posScale().
    const scale = posScale();
    const px = Math.round(screenX * scale);
    const py = Math.round(screenY * scale);

    const target = await resolveRedockWindow(s, ourLabel, px, py, expectedTarget);
    if (!target) return;

    const targetWs = await resolveTargetWorkspace(s, target.window_id);
    if (!targetWs) return;
    const targetTabId = targetWs.activetabid;
    const targetWsId = targetWs.oid;
    if (!targetTabId || !targetWsId) {
        console.warn(
            "[floating-pane] target window has no active tab — skipping redock",
        );
        return;
    }

    // Source identifiers — the floater's only-tab + only-block.
    const sourceTabId = env.tabId();
    const sourceWs = env.ws();
    if (!sourceTabId || !sourceWs) return;
    const sourceWsId = sourceWs.oid;
    // Non-reactive read: `useMuxObjectValue` would register an
    // `onCleanup` against the current reactive owner, but we're inside
    // an async mouseup callback with no owner — the refCount would
    // never get decremented and we'd leak a Tab subscription per drop.
    // The pane the floater holds, with all of its tabs when it was torn off
    // as a stack (floater-panes.ts).
    const pane = floaterPanes(sourceTabId)[0];
    if (!pane) {
        console.warn(
            "[floating-pane] floater has no block to redock — skipping",
        );
        return;
    }

    // Phase 4b — use the ghost pre-captured in onMouseUp (before
    // clear_floating_redock_hover cleared it). If the resolved target
    // window differs from the captured window, fall back to empty ghost
    // (InsertNode path). Clear after consuming so a subsequent drag
    // can't accidentally reuse stale state.
    const ghost =
        s.capturedGhostForDrop != null && s.capturedGhostForWindow === target.label
            ? await s.capturedGhostForDrop
            : {};
    s.capturedGhostForDrop = null;
    s.capturedGhostForWindow = null;

    try {
        await redockFloaterPane(
            pane,
            { tabId: sourceTabId, workspaceId: sourceWsId },
            { tabId: targetTabId, workspaceId: targetWsId },
            { blockId: ghost.block_id, dir: ghost.dir },
        );
        // After successful redock, source tab.blockids empties
        // → the auto-close watcher dismisses the floater.
        // Signal "a redock happened" so the caller keeps the redock
        // guard open through the floater's close + node teardown.
        return true;
    } catch (e) {
        console.error("[floating-pane] RedockFloatingPane failed", e);
    }
    return false;
}

// Double-click on the header maximizes or restores the floater, as on any
// window's title bar, instead of the docked panes' layout magnify (which in
// a floater only magnifies its one pane inside its own window). Capture
// phase, so it runs before the header's own dblclick and stops it.
// SPEC_FLOATING_PANE_WINDOW_CONTROLS_REFINEMENTS_2026_10_16 §4.5.
function onHeaderDblClick(env: FloaterEnv, e: MouseEvent): void {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement | null;
    if (!isFloaterHeaderMovePress(target)) return;
    const tid = env.tabId();
    const blockId = tid ? floaterPaneBlockId(tid) : undefined;
    if (!blockId) return;
    e.stopPropagation();
    getApi().windows.toggleFloatingMaximize(env.label, blockId).catch(console.error);
}

// Native Wayland: hand a header press that has moved to the compositor.
// One IPC; the page sees no motion until the compositor lets go.
function onCompositorPressMove(s: FloaterDragState, env: FloaterEnv, e: MouseEvent): void {
    if (!s.compositorPress) return;
    // The button came up somewhere the page didn't see the mouseup.
    if ((e.buttons & 1) === 0) {
        s.compositorPress = null;
        return;
    }
    const dx = Math.abs(e.clientX - s.compositorPress.x);
    const dy = Math.abs(e.clientY - s.compositorPress.y);
    if (dx < COMPOSITOR_MOVE_THRESHOLD_PX && dy < COMPOSITOR_MOVE_THRESHOLD_PX) return;
    s.compositorPress = null;
    getApi().windows.startDrag(env.label).catch(() => {});
}

// Windows, a maximized floater: start the host's move loop (which
// restores it under the cursor) once the press has moved.
function onWin32MaximizedPressMove(s: FloaterDragState, env: FloaterEnv, e: MouseEvent): void {
    if (!s.win32MaximizedPress) return;
    if ((e.buttons & 1) === 0) {
        s.win32MaximizedPress = null;
        return;
    }
    const dx = Math.abs(e.clientX - s.win32MaximizedPress.x);
    const dy = Math.abs(e.clientY - s.win32MaximizedPress.y);
    if (dx < COMPOSITOR_MOVE_THRESHOLD_PX && dy < COMPOSITOR_MOVE_THRESHOLD_PX) return;
    s.win32MaximizedPress = null;
    startWin32Drag(s, env);
}

/** Installs everything that moves, resizes and docks the floater: edge resize, the header drag on this platform,
 *  the host events, the dblclick maximize. Call inside onMount; the cleanups ride on its owner. */
function installFloaterWindowDrag(env: FloaterEnv): void {
    const s = createDragState();
    if (env.label.startsWith("floating-")) installEdgeResize(env.label);

    const stopCancelListener = listenDragCancelled(s, env);
    const stopEndedListener = listenDragEnded(s, env);
    const stopHoverStateListener = listenHoverState(s);
    const unlistenMouseMove = installMouseMoveTracking(s, env);

    // Both mousedown and mouseup are capture-phase so they always fire
    // before any child stopPropagation call can suppress them. mousedown
    // must be capture to pre-empt pragmatic-dnd's dragstart; mouseup must
    // be capture to ensure dragging is always reset (a bubble-phase mouseup
    // blocked by a child would silently leave dragging=true).
    const onDblClick = (e: MouseEvent) => onHeaderDblClick(env, e);
    const onMouseDown = (e: MouseEvent) => onHeaderMouseDown(s, env, e);
    const onMouseUp = (e: MouseEvent) => onHeaderMouseUp(s, env, e);
    const onCompositorMove = (e: MouseEvent) => onCompositorPressMove(s, env, e);
    const onWin32PressMove = (e: MouseEvent) => onWin32MaximizedPressMove(s, env, e);

    document.addEventListener("dblclick", onDblClick, true);
    onCleanup(() => document.removeEventListener("dblclick", onDblClick, true));
    document.addEventListener("mousedown", onMouseDown, true);
    document.addEventListener("mouseup", onMouseUp, true);
    document.addEventListener("mousemove", onCompositorMove, true);
    document.addEventListener("mousemove", onWin32PressMove, true);

    onCleanup(() => {
        s.cleaned = true;
        document.removeEventListener("mousedown", onMouseDown, true);
        document.removeEventListener("mouseup", onMouseUp, true);
        document.removeEventListener("mousemove", onCompositorMove, true);
        document.removeEventListener("mousemove", onWin32PressMove, true);
        stopCancelListener();
        stopEndedListener();
        stopHoverStateListener();
        s.stopHoverHeartbeat();
        unlistenMouseMove();
    });
}

// Auto-close the floating window when its only pane is closed.
// The Workspace MuxObj has `tabids` but NO `blockids` field — the
// block-membership signal lives on the Tab (`tab.blockids`, see
// the `Tab` type in `frontend/types/srv-types.d.ts`). We subscribe to
// the active tab and trigger close as soon as its blockids array
// transitions from non-empty → empty. The `hadBlocks` latch avoids
// closing on the brief empty state during initial workspace load.
//
// useMuxObjectValue installs an onCleanup tied to the surrounding
// reactive owner — calling it inside createEffect refreshes the
// subscription whenever tabId changes (the prior effect run's
// cleanup decrements the previous refcount).
function useCloseWhenTabEmpties(tabId: typeof atoms.activeTabId, windowLabel: () => string): void {
    let hadBlocks = false;
    // `redockInProgress` is a reactive module-level guard (floating-redock-guard.ts)
    // shared with the Redock button, so the effect re-runs when it goes false again
    // after the RPC completes, and the floater closes after a dock.
    createEffect(() => {
        const tid = tabId();
        if (!tid) return;
        const [tab] = MOS.useMuxObjectValue<Tab>(MOS.makeORef("tab", tid));
        const t = tab();
        if (!t) return;
        const blockids = t.blockids ?? [];
        if (blockids.length > 0) {
            hadBlocks = true;
        } else if (hadBlocks && !redockInProgress()) {
            // Skip close while a RedockFloatingPane RPC is in-flight —
            // the backend broadcasts the Tab update before the RPC response
            // arrives, so the watcher would otherwise destroy the window mid-op.
            const label = windowLabel();
            if (label) {
                getApi()
                    .closeWindowByLabel(label)
                    .catch((e) =>
                        console.error(
                            "[floating-pane] auto-close on empty tab failed",
                            e,
                        ),
                    );
            }
        }
    });
}

function FloatingPaneWorkspaceElem(): JSX.Element {
    const tabId = atoms.activeTabId;
    const ws = atoms.workspace;

    const windowLabel = createMemo(() => {
        const params = new URLSearchParams(window.location.search);
        return params.get("windowLabel") ?? "";
    });

    // "Always on top": re-apply a stored tack to this (fresh) window.
    // SPEC_FLOATING_PANE_ALWAYS_ON_TOP_2026_09_27 §6.2.
    useReapplyFloatingOnTop(windowLabel, () => {
        const id = tabId();
        return id ? floaterPaneBlockId(id) : undefined;
    });

    useCloseWhenTabEmpties(tabId, windowLabel);

    // Pane-header drag — Windows: Win32BeginMoveTask host-side loop (PR #1276).
    // macOS + Linux: JS-driven get/set_window_position polling (see the
    // `jsDrivenDrag` note in onMount for why each platform needs it). On
    // macOS/Linux a `mousemove` listener emits `update_floating_redock_hover`
    // for the drop-target highlight and drives set_window_position; on Windows
    // that emission lives inside Win32BeginMoveTask.
    //
    // `preventDefault()` on the qualifying mousedown is load-bearing:
    // it suppresses the HTML5 dragstart pragmatic-dnd would otherwise
    // use to initiate a pane tear-off (the header `draggable()` that
    // TileLayout.core.tsx's `DisplayNode` registers on every platform).
    // Without it, dragging a floater's header would tear the block off
    // into ANOTHER floating window — the "double tear-off" bug.
    //
    // See docs/analysis/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md.
    onMount(() => {
        // JS-driven floater drag (get/set_window_position polling) instead of the
        // host-side native move loop, on the platforms where the native loop
        // breaks hover/redock:
        //   - macOS: BeginWindowDrag needs a patched libcef absent in dev builds
        //     (PR #1308).
        //   - Linux on X11/XWayland: BeginWindowDrag → _NET_WM_MOVERESIZE hands
        //     input to the window manager for the drag, so the renderer receives
        //     NO DOM mousemove/mouseup — update_floating_redock_hover never fires
        //     (no landing ghosts) and the hasMoved gate never trips (no redock).
        //   - Linux on native Wayland is the exception, below (compositorPress):
        //     no client can place its own window, so set_window_position does
        //     nothing there and only the compositor's move works.
        // Both use the JS-driven path; it also inherits the dwell + velocity gate
        // (PR #1249). Windows keeps Win32BeginMoveTask (a host-side loop that DOES
        // emit hover while the renderer is dark).
        //
        // Computed HERE (inside onMount), not at module scope: platformutil's
        // PLATFORM defaults to "darwin" until setPlatform() runs during app init,
        // so a module-scope read would capture "darwin" and force this true on
        // every platform (incl. Windows). By onMount, setPlatform() has run.
        const jsDrivenDrag = isMacOS() || isLinux();
        installFloaterWindowDrag({ label: windowLabel(), windowLabel, tabId, ws, jsDrivenDrag });
    });

    return (
        <div class="floating-pane-workspace flex flex-col w-full flex-grow overflow-hidden">
            {/* The torn-off block lives in the new workspace's active
                tab. Render that tab's TabContent only — no per-tab loop
                (there's exactly one tab) and no tab bar / widgets bar /
                status bar to surround it. The block renders its standard
                `BlockFrame_Header` which serves as both the title bar
                and the action surface — exactly as it appears when
                docked. */}
            <div
                class="flex flex-row flex-grow overflow-hidden"
                style={{ "min-height": 0 }}
            >
                <ErrorBoundary>
                    <Show
                        when={ws() && tabId()}
                        fallback={<CenteredDiv>Loading pane…</CenteredDiv>}
                    >
                        <ErrorBoundary>
                            <TabContent tabId={tabId()} />
                        </ErrorBoundary>
                    </Show>
                    <ModalsRenderer />
                </ErrorBoundary>
            </div>
        </div>
    );
}

export { FloatingPaneWorkspaceElem as FloatingPaneWorkspace };