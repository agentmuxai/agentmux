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
 *    `theme.scss:97`) as its sole chrome.
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

import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import { isLinux, isMacOS, isWindows } from "@/util/platformutil";
import { FLOATER_EDGE_RESIZE_BORDER } from "@/app/workspace/floater-resize";
import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { TabContent } from "@/app/tab/tabcontent";
import { WorkspaceService } from "@/store/services";
import { atoms, getApi } from "@/store/global";
import * as WOS from "@/store/wos";
import { Show, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";

import { REDOCK_DWELL_MS, REDOCK_VELOCITY_PX_PER_S } from "./floating-pane-constants";
import { initFloaterCtrlWheel } from "./floater-ctrl-wheel";
import "./floating-pane-workspace.scss";

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

function FloatingPaneWorkspaceElem(): JSX.Element {
    const tabId = atoms.activeTabId;
    const ws = atoms.workspace;

    const windowLabel = createMemo(() => {
        const params = new URLSearchParams(window.location.search);
        return params.get("windowLabel") ?? "";
    });

    // Auto-close the floating window when its only pane is closed.
    // The Workspace WaveObj has `tabids` but NO `blockids` field — the
    // block-membership signal lives on the Tab (`tab.blockids`, see
    // `frontend/types/gotypes.d.ts:1491`). We subscribe to the active
    // tab and trigger close as soon as its blockids array transitions
    // from non-empty → empty. The `hadBlocks` latch avoids closing on
    // the brief empty state during initial workspace load.
    //
    // useWaveObjectValue installs an onCleanup tied to the surrounding
    // reactive owner — calling it inside createEffect refreshes the
    // subscription whenever tabId changes (the prior effect run's
    // cleanup decrements the previous refcount).
    let hadBlocks = false;
    // Signal so createEffect re-runs when redockInProgress changes — a plain
    // JS boolean is non-reactive, so the effect would never see the flag go
    // false again after the RPC completes, leaving the floater open after dock.
    const [redockInProgress, setRedockInProgress] = createSignal(false);
    createEffect(() => {
        const tid = tabId();
        if (!tid) return;
        const [tab] = WOS.useWaveObjectValue<Tab>(WOS.makeORef("tab", tid));
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

    // Ctrl+Wheel recovery. CEF swallows Ctrl+Wheel in a child-HWND browser, so
    // no wheel event with `ctrlKey` ever reaches this window's DOM and none of
    // the per-view zoom handlers can fire. The host intercepts the Win32
    // message and forwards it; this re-dispatches it as the DOM event that was
    // swallowed. See floater-ctrl-wheel.ts and floater_wheel.rs.
    onMount(() => {
        let dispose: (() => void) | null = null;
        let disposed = false;
        void initFloaterCtrlWheel().then((fn) => {
            // The listener resolves async; if this component unmounted first,
            // tear it down immediately rather than leaking it.
            if (disposed) fn();
            else dispose = fn;
        });
        onCleanup(() => {
            disposed = true;
            dispose?.();
        });
    });

    // Pane-header drag — Windows: Win32BeginMoveTask host-side loop (PR #1276).
    // macOS + Linux: JS-driven get/set_window_position polling (see the
    // `jsDrivenDrag` note in onMount for why each platform needs it). On
    // macOS/Linux a `mousemove` listener emits `update_floating_redock_hover`
    // for the drop-target highlight and drives set_window_position; on Windows
    // that emission lives inside Win32BeginMoveTask.
    //
    // `preventDefault()` on the qualifying mousedown is load-bearing:
    // it suppresses the HTML5 dragstart pragmatic-dnd would otherwise
    // use to initiate a pane tear-off (TileLayout.win32.tsx:443-471).
    // Without it, dragging a floater's header would tear the block off
    // into ANOTHER floating window — the "double tear-off" bug.
    //
    // See docs/analysis/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md.
    onMount(() => {
        const INTERACTIVE_SELECTOR =
            "button, a, input, select, textarea, [role='button']";
        const HEADER_SELECTOR = '[data-role="block-header"]';

        // JS-driven floater drag (get/set_window_position polling) instead of the
        // host-side native move loop, on the platforms where the native loop
        // breaks hover/redock:
        //   - macOS: BeginWindowDrag needs a patched libcef absent in dev builds
        //     (PR #1308).
        //   - Linux: BeginWindowDrag → _NET_WM_MOVERESIZE hands input to the X11/
        //     Wayland compositor for the drag, so the renderer receives NO DOM
        //     mousemove/mouseup — update_floating_redock_hover never fires (no
        //     landing ghosts) and the hasMoved gate never trips (no redock).
        // Both use the JS-driven path; it also inherits the dwell + velocity gate
        // (PR #1249). Windows keeps Win32BeginMoveTask (a host-side loop that DOES
        // emit hover while the renderer is dark).
        //
        // Computed HERE (inside onMount), not at module scope: platformutil's
        // PLATFORM defaults to "darwin" until setPlatform() runs during app init,
        // so a module-scope read would capture "darwin" and force this true on
        // every platform (incl. Windows). By onMount, setPlatform() has run.
        const jsDrivenDrag = isMacOS() || isLinux();

        let dragging = false;
        // On Windows: coords saved here by onMouseUp; consumed by the
        // window_drag_ended handler which carries host cursor_x/cursor_y
        // (eliminating the async ordering race with DOM mouseup).
        // On non-Windows: tryRedockAtCursor is called directly from onMouseUp
        // when hasMoved=true, so pendingRedockCoords is only a fallback.
        let pendingRedockCoords: { x: number; y: number } | null = null;
        // Non-Windows only: set by onMouseMove when dragging. Gates
        // tryRedockAtCursor in onMouseUp so a plain header click (no pixel
        // motion) doesn't false-redock when the floater overlaps another window.
        let hasMoved = false;
        // JS-driven drag state (macOS + Linux — see jsDrivenDrag above):
        // get/set_window_position polling instead of the host native move loop.
        // Only read when jsDrivenDrag is true.
        let jsDragClickScreenX = 0;
        let jsDragClickScreenY = 0;
        let jsDragInitWinX = 0;
        let jsDragInitWinY = 0;
        let jsDragLatestScreenX = 0;
        let jsDragLatestScreenY = 0;
        let jsDragSetPosInFlight = false;
        let jsDragPendingPos: { x: number; y: number } | null = null;
        let jsDragMouseDownId = 0;
        // Per-drag session counter: incremented on every drag start and on cancel.
        // Guards the update_floating_redock_hover IPC .then() against stale responses
        // from a prior drag session arriving during a new drag (or after a cancel).
        let dragSessionId = 0;
        // Dwell + velocity gate: prevents accidental redock when the cursor
        // transits over another window at speed. Arms hover only after the cursor
        // stays near the same target window for REDOCK_DWELL_MS ms at
        // ≤ REDOCK_VELOCITY_PX_PER_S CSS-px/s. All vars hoisted to drag scope so
        // a second drag never inherits state from the first.
        let hoverArmed = false;
        // Preserves arm state across the Windows mouseup/window_drag_ended race:
        // onMouseUp sets pendingRedockArmed=hoverArmed before clearing hoverArmed,
        // so window_drag_ended can safely use either flag.
        let pendingRedockArmed = false;
        // Velocity sampling state (non-Windows mousemove path).
        let dwellLastMoveSampleAt = 0;
        let dwellLastMoveSampleX = 0;
        let dwellLastMoveSampleY = 0;
        let dwellSlowSince: number | null = null;
        // Per-target dwell (non-Windows): reset arming when IPC returns a new target.
        let dwellLastArmedTarget: string | null = null;
        // Non-Windows: wall-clock time when IPC first confirmed the current target.
        // Used as fallback when cursor holds still after first confirmation (no 2nd IPC).
        let dwellCurrentConfirmedAt: number | null = null;
        // Non-Windows: true once update_floating_redock_hover returned a non-null target
        // (indicator is showing on the backend). Separate from hoverArmed so the velocity
        // gate can clear the indicator even before the full dwell interval has elapsed.
        let indicatorShowing = false;
        // Per-target dwell (Windows): driven by floating-redock:hover-state events.
        let dwellCurrentHoverTarget: string | null = null;
        let dwellHoverTargetFirstSeenAt: number | null = null;
        // Velocity sampling for Windows (hover-state events carry cursor_x/y).
        let dwellWinLastSampleAt = 0;
        let dwellWinLastSampleX = 0;
        let dwellWinLastSampleY = 0;
        // redockInProgress is a signal at component scope (above this onMount)
        // so createEffect re-runs when it changes.
        // Sentinel for the listenEvent .then() race: if the component unmounts
        // before the Promise resolves, the .then() immediately calls unlisten.
        let cleaned = false;
        // Phase 4b — ghost pre-captured in onMouseUp before clear_floating_redock_hover
        // broadcasts the hover-state null event (which triggers clearPlaceholder on the
        // target renderer and would wipe the ghost before tryRedockAtCursorInner reads it).
        let capturedGhostForDrop: Promise<{ block_id?: string; dir?: number }> | null = null;
        let capturedGhostForWindow: string | null = null;

        const label = windowLabel();

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
        if (label.startsWith("floating-")) {
            // Invisible edge grab-band depth (CSS px). Shared with the browser
            // pane's web-content inset so they can't drift — see floater-resize.ts.
            const RESIZE_BORDER = FLOATER_EDGE_RESIZE_BORDER;
            const MIN_W = 240;
            const MIN_H = 140;
            // Direction codes 1..8: LEFT, RIGHT, TOP, TOP-LEFT, TOP-RIGHT,
            // BOTTOM, BOTTOM-LEFT, BOTTOM-RIGHT (WMSZ_* ordering).
            const edgeDirection = (e: { clientX: number; clientY: number }): number => {
                const w = window.innerWidth;
                const h = window.innerHeight;
                const left = e.clientX <= RESIZE_BORDER;
                const right = e.clientX >= w - RESIZE_BORDER;
                const top = e.clientY <= RESIZE_BORDER;
                const bottom = e.clientY >= h - RESIZE_BORDER;
                if (top && left) return 4;
                if (top && right) return 5;
                if (bottom && left) return 7;
                if (bottom && right) return 8;
                if (left) return 1;
                if (right) return 2;
                if (top) return 3;
                if (bottom) return 6;
                return 0;
            };
            const CURSORS = ["", "ew-resize", "ew-resize", "ns-resize",
                "nwse-resize", "nesw-resize", "ns-resize", "nesw-resize", "nwse-resize"];

            let resizing = false;
            let dir = 0;
            let startX = 0; // screen px at pointerdown
            let startY = 0;
            let startRect = { x: 0, y: 0, w: 0, h: 0 }; // physical px (Windows) / DIP (macOS, Linux)

            // Coalesce moves the same way the header MOVE does: one IPC in
            // flight at a time, last position wins (no timers/RAF).
            let rectInFlight = false;
            let pendingRect: { x: number; y: number; width: number; height: number } | null = null;
            const sendRect = (r: { x: number; y: number; width: number; height: number }): void => {
                if (rectInFlight) { pendingRect = r; return; }
                rectInFlight = true;
                invokeCommand("set_window_rect", { label, ...r })
                    .catch(() => {})
                    .finally(() => {
                        rectInFlight = false;
                        if (pendingRect) { const p = pendingRect; pendingRect = null; sendRect(p); }
                    });
            };

            const onPointerDown = (e: PointerEvent): void => {
                if (e.button !== 0) return;
                const d = edgeDirection(e);
                if (d === 0) return;
                // Win the event over the header-drag + content handlers, and
                // stop text selection.
                e.preventDefault();
                e.stopImmediatePropagation();
                dir = d;
                startX = e.screenX;
                startY = e.screenY;
                const target = e.target as Element;
                // Take capture synchronously so moves keep flowing even as the
                // cursor leaves the window; then fetch the start rect.
                try { target.setPointerCapture(e.pointerId); } catch { /* ignore */ }
                invokeCommand<{ x: number; y: number; width: number; height: number }>(
                    "get_window_rect",
                    { label },
                ).then((r) => {
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
                    const d = edgeDirection(e);
                    document.body.style.cursor = d ? CURSORS[d] : "";
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
                if (w < MIN_W) { if (left) x -= MIN_W - w; w = MIN_W; }
                if (h < MIN_H) { if (top) y -= MIN_H - h; h = MIN_H; }
                sendRect({ x, y, width: w, height: h });
            };

            const onPointerUp = (e: PointerEvent): void => {
                if (!resizing) return;
                resizing = false;
                try { (e.target as Element).releasePointerCapture(e.pointerId); } catch { /* ignore */ }
                document.body.style.cursor = "";
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
            });
        }

        // dragId guards the drain: if a new mousedown starts (incrementing
        // jsDragMouseDownId) while a set_window_position is in-flight, the old
        // drain's .finally discards jsDragPendingPos instead of applying a delta
        // from the previous drag's origin to the new drag's jsDragInitWinX/Y.
        const jsDragSendPos = (dragId: number, x: number, y: number): void => {
            if (jsDragSetPosInFlight) {
                jsDragPendingPos = { x, y };
                return;
            }
            jsDragSetPosInFlight = true;
            invokeCommand("set_window_position", { x, y, label })
                .catch(() => {})
                .finally(() => {
                    jsDragSetPosInFlight = false;
                    if (jsDragPendingPos && dragId === jsDragMouseDownId) {
                        const { x: nx, y: ny } = jsDragPendingPos;
                        jsDragPendingPos = null;
                        jsDragSendPos(dragId, nx, ny);
                    } else {
                        jsDragPendingPos = null;
                    }
                });
        };

        const onMouseDown = (e: MouseEvent) => {
            if (e.button !== 0) return;
            const target = e.target as HTMLElement | null;
            if (!target) return;
            if (!target.closest(HEADER_SELECTOR)) return;
            if (target.closest(INTERACTIVE_SELECTOR)) return;

            // Load-bearing: blocks pragmatic-dnd's HTML5 dragstart, preventing
            // the double-tear-off regression (ANALYSIS_FLOATING_PANE_HEADER_DRAG
            // §"Tear-off conflict").
            e.preventDefault();

            if (jsDrivenDrag) {
                // JS-driven drag (macOS + Linux). macOS: BeginWindowDrag needs a
                // patched libcef absent in dev builds. Linux: BeginWindowDrag →
                // _NET_WM_MOVERESIZE makes the compositor swallow all input, so the
                // renderer goes dark (no mousemove → no hover ghosts; no mouseup
                // gate → no redock). Both use get/set_window_position polling.
                jsDragMouseDownId += 1;
                dragSessionId += 1;
                const myId = jsDragMouseDownId;
                jsDragClickScreenX = e.screenX;
                jsDragClickScreenY = e.screenY;
                jsDragLatestScreenX = e.screenX;
                jsDragLatestScreenY = e.screenY;
                hasMoved = false;
                pendingRedockCoords = null;
                hoverArmed = false;
                pendingRedockArmed = false;
                dwellLastMoveSampleAt = 0;
                dwellLastMoveSampleX = 0;
                dwellLastMoveSampleY = 0;
                dwellSlowSince = null;
                dwellLastArmedTarget = null;
                dwellCurrentConfirmedAt = null;
                indicatorShowing = false;
                dwellCurrentHoverTarget = null;
                dwellHoverTargetFirstSeenAt = null;
                dwellWinLastSampleAt = 0;
                dwellWinLastSampleX = 0;
                dwellWinLastSampleY = 0;
                invokeCommand<{ x: number; y: number }>("get_window_position", { label })
                    .then((pos) => {
                        if (myId !== jsDragMouseDownId) return;
                        jsDragInitWinX = pos.x;
                        jsDragInitWinY = pos.y;
                        const movedDuringIPC =
                            jsDragLatestScreenX !== jsDragClickScreenX ||
                            jsDragLatestScreenY !== jsDragClickScreenY;
                        if (movedDuringIPC) {
                            // Motion happened before dragging was armed; set
                            // hasMoved so onMouseUp's gate allows tryRedockAtCursor.
                            hasMoved = true;
                            const scale = posScale();
                            jsDragSendPos(
                                myId,
                                jsDragInitWinX + Math.round((jsDragLatestScreenX - jsDragClickScreenX) * scale),
                                jsDragInitWinY + Math.round((jsDragLatestScreenY - jsDragClickScreenY) * scale),
                            );
                        }
                        dragging = true;
                    })
                    .catch(() => {});
                return;
            }

            // Windows only now (macOS + Linux returned via jsDrivenDrag above).
            // Win32BeginMoveTask manual loop (zero per-move IPC, no DPR math).
            // The host owns motion + capture and emits update_floating_redock_hover
            // itself while the renderer is dark; renderer sees mouseup via the
            // dispatched WM_LBUTTONUP balance (PR #1181 §5.1).
            invokeCommand("start_window_drag", { label }).catch(() => {});
            dragging = true;
            hasMoved = false;
            pendingRedockCoords = null;
            hoverArmed = false;
            pendingRedockArmed = false;
            dwellLastMoveSampleAt = 0;
            dwellLastMoveSampleX = 0;
            dwellLastMoveSampleY = 0;
            dwellSlowSince = null;
            dwellLastArmedTarget = null;
            dwellCurrentConfirmedAt = null;
            indicatorShowing = false;
            dwellCurrentHoverTarget = null;
            dwellHoverTargetFirstSeenAt = null;
            dwellWinLastSampleAt = 0;
            dwellWinLastSampleX = 0;
            dwellWinLastSampleY = 0;
            dragSessionId += 1;
        };

        const onMouseUp = (e: MouseEvent) => {
            // Invalidate any in-flight JS-driven get_window_position IPC so a race
            // where mouseup fires before the promise resolves doesn't arm dragging.
            if (jsDrivenDrag) jsDragMouseDownId += 1;
            if (!dragging) return;
            dragging = false;
            // On macOS/Linux, paneRect() returns unchanged client coords after a
            // window move, so browser-view's syncPosition dedupe guard skips the
            // browser_pane_resize re-send. Signal it to force one so the
            // NativeWidgetMacNSWindow overlay repositions to the new window frame.
            if (jsDrivenDrag) {
                window.dispatchEvent(
                    new CustomEvent("floating-pane-js-drag-ended", { detail: { label } }),
                );
            }
            // Phase 4b — dispatch get_floating_redock_target BEFORE clear_floating_redock_hover.
            // Both are fire-and-forget IPCs from the same floater renderer → CEF backend
            // channel (FIFO). The ghost read queues ahead of the event broadcast that
            // triggers clearPlaceholder on the target renderer. This avoids the cross-
            // process race where the target's set_floating_redock_target(null) arrives
            // before tryRedockAtCursorInner's delayed call to get_floating_redock_target.
            const preGhostWindow = dwellCurrentHoverTarget;
            capturedGhostForWindow = preGhostWindow;
            capturedGhostForDrop = preGhostWindow
                ? invokeCommand<{ block_id?: string; dir?: number }>(
                      "get_floating_redock_target",
                      { window_label: preGhostWindow },
                  ).catch(() => ({}))
                : Promise.resolve({});
            invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
            if (isWindows()) {
                // On Windows: preserve arm state before clearing — window_drag_ended
                // may arrive before or after DOM mouseup (separate CEF IPC channels).
                // If ended arrives first: hoverArmed is still true and wins.
                // If mouseup arrives first: pendingRedockArmed carries it for ended.
                // Point-in-time fallback: if user held still over a target until the
                // dwell threshold elapsed but no further hover-state event fired to set
                // hoverArmed, check the wall-clock directly at mouseup time.
                const nowMs = performance.now();
                pendingRedockArmed = hoverArmed ||
                    (dwellCurrentHoverTarget !== null &&
                     dwellHoverTargetFirstSeenAt !== null &&
                     nowMs - dwellHoverTargetFirstSeenAt >= REDOCK_DWELL_MS) ||
                    // Hold-still after fast entry: velocity gate preserved the target
                    // but cleared the timer; cursor then stopped so no slow event
                    // restarted it. Check time-since-last-event as dwell proxy.
                    (dwellCurrentHoverTarget !== null &&
                     dwellHoverTargetFirstSeenAt === null &&
                     dwellWinLastSampleAt > 0 &&
                     nowMs - dwellWinLastSampleAt >= REDOCK_DWELL_MS);
                hoverArmed = false;
                pendingRedockCoords = { x: e.screenX, y: e.screenY };
            } else if (hasMoved) {
                // Non-Windows (macOS + Linux): the JS-driven path keeps the
                // renderer receiving mousemove+mouseup normally. Only attempt
                // redock if the dwell gate armed and motion confirmed.
                // Arm conditions (any one suffices):
                // 1. hoverArmed: second IPC confirmed same target (full dwell cycle).
                // 2. dwellSlowSince elapsed: cursor slowed and stopped before
                //    the IPC could fire (stopped during the slow-motion window);
                //    tryRedockAtCursorInner handles the no-target case gracefully.
                // 3. dwellCurrentConfirmedAt elapsed: first IPC confirmed the target;
                //    user has now held still over it for a full REDOCK_DWELL_MS.
                //    (indicatorShowing intentionally excluded — arming on first IPC
                //    confirmation lets slow desktop transits dock without per-target
                //    dwell; dwellCurrentConfirmedAt is the spec-correct check.)
                const nowMs = performance.now();
                const armed = hoverArmed ||
                    (dwellSlowSince !== null &&
                     nowMs - dwellSlowSince >= REDOCK_DWELL_MS) ||
                    (dwellCurrentConfirmedAt !== null &&
                     nowMs - dwellCurrentConfirmedAt >= REDOCK_DWELL_MS);
                // Invalidate in-flight IPC .then() so it cannot re-arm hoverArmed
                // after this mouseup has already consumed and cleared the state
                // (reagent P1 — stale IPC re-arm → spurious window_drag_ended dock).
                dragSessionId += 1;
                hoverArmed = false;
                indicatorShowing = false;
                // Clear non-Windows dwell state so window_drag_ended (Linux race)
                // cannot see stale dwellCurrentConfirmedAt and fire a second
                // tryRedockAtCursor on the already-redocked block (reagent P1).
                dwellCurrentConfirmedAt = null;
                dwellCurrentHoverTarget = null;
                dwellHoverTargetFirstSeenAt = null;
                if (armed) void tryRedockAtCursor(e.screenX, e.screenY);
            }
        };

        // Helper: safe unlisten registration using the `cleaned` sentinel.
        // If the component unmounts before listenEvent's Promise resolves,
        // the .then() calls unlisten immediately rather than storing into a
        // stale closure — prevents accumulated listener leaks across tear-off
        // / remount cycles.
        const safeListenEvent = <T,>(
            event: string,
            handler: (payload: T) => void,
        ): (() => void) => {
            let unlisten: (() => void) | null = null;
            listenEvent<T>(event, handler).then(u => {
                if (cleaned) { u(); } else { unlisten = u; }
            }).catch(() => {});
            return () => { unlisten?.(); };
        };

        // Esc-cancel from host — clear pending redock coords to prevent a
        // spurious dock attempt on the next mouseup.
        const stopCancelListener = safeListenEvent<{ label: string }>(
            "window_drag_cancelled",
            (ev) => {
                if (!ev.label || ev.label === label) {
                    if (jsDrivenDrag) jsDragMouseDownId += 1;
                    dragSessionId += 1;
                    dragging = false;
                    hasMoved = false;
                    pendingRedockCoords = null;
                    hoverArmed = false;
                    pendingRedockArmed = false;
                    dwellSlowSince = null;
                    dwellLastArmedTarget = null;
                    dwellCurrentConfirmedAt = null;
                    indicatorShowing = false;
                    dwellCurrentHoverTarget = null;
                    dwellHoverTargetFirstSeenAt = null;
                    dwellLastMoveSampleAt = 0;
                    dwellLastMoveSampleX = 0;
                    dwellLastMoveSampleY = 0;
                    dwellWinLastSampleAt = 0;
                    dwellWinLastSampleX = 0;
                    dwellWinLastSampleY = 0;
                }
            },
        );

        // Host signals drag loop ended. Always resets dragging and clears hover.
        // On Windows: cursor_x/cursor_y (physical px) are included so the handler
        // is self-contained — eliminates the ordering race where DOM mouseup and
        // this event arrive via separate CEF IPC channels (relative order not
        // guaranteed). On non-Windows: falls back to pendingRedockCoords set by
        // onMouseUp if the OS delivered a DOM mouseup (BeginWindowDrag case).
        const stopEndedListener = safeListenEvent<{
            label: string;
            moved: boolean;
            cursor_x?: number;
            cursor_y?: number;
        }>(
            "window_drag_ended",
            (ev) => {
                if (!ev.label || ev.label !== label) return;
                dragging = false;
                // Capture arm state before clearing — handles the Windows race where
                // this event can arrive before or after DOM mouseup (separate CEF IPC
                // channels). Before mouseup: hoverArmed is still true. After mouseup:
                // onMouseUp already moved it to pendingRedockArmed.
                // Point-in-time fallback: if the user held still over a target until
                // the dwell elapsed but no event fired to set hoverArmed (or mouseup
                // raced ahead and set pendingRedockArmed via the fallback there), the
                // wall-clock check here covers the window_drag_ended ordering leg.
                const nowMs = performance.now();
                const armedAtEnd = pendingRedockArmed || hoverArmed ||
                    (dwellCurrentHoverTarget !== null &&
                     dwellHoverTargetFirstSeenAt !== null &&
                     nowMs - dwellHoverTargetFirstSeenAt >= REDOCK_DWELL_MS) ||
                    // Hold-still after fast entry (Windows): same fallback as onMouseUp.
                    (dwellCurrentHoverTarget !== null &&
                     dwellHoverTargetFirstSeenAt === null &&
                     dwellWinLastSampleAt > 0 &&
                     nowMs - dwellWinLastSampleAt >= REDOCK_DWELL_MS) ||
                    (dwellCurrentConfirmedAt !== null &&
                     nowMs - dwellCurrentConfirmedAt >= REDOCK_DWELL_MS);
                hoverArmed = false;
                indicatorShowing = false;
                pendingRedockArmed = false;
                // Phase 4b — pre-capture ghost if onMouseUp hasn't already done so
                // (window_drag_ended can arrive before DOM mouseup on Windows — separate
                // CEF IPC channels, documented above). Guards against double-consume:
                // get_floating_redock_target removes the entry atomically, so the
                // second caller would get {} anyway, but skip the IPC entirely.
                if (!capturedGhostForDrop) {
                    const preGhostWindow = dwellCurrentHoverTarget;
                    capturedGhostForWindow = preGhostWindow;
                    capturedGhostForDrop = preGhostWindow
                        ? invokeCommand<{ block_id?: string; dir?: number }>(
                              "get_floating_redock_target",
                              { window_label: preGhostWindow },
                          ).catch(() => ({}))
                        : Promise.resolve({});
                }
                // Always clear hover — safety net for non-Windows where onMouseUp
                // may not have fired (BeginWindowDrag absorbs the release).
                invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
                if (ev.moved && !cleaned && armedAtEnd) {
                    // Prefer host-provided cursor coords (physical px → CSS px via
                    // posScale). On Windows these are always present. On non-Windows
                    // fall back to pendingRedockCoords saved by onMouseUp if the OS
                    // delivered a DOM mouseup; otherwise redock is F3-pending.
                    const coords = (() => {
                        if (typeof ev.cursor_x === "number" && typeof ev.cursor_y === "number") {
                            const scale = posScale();
                            return { x: ev.cursor_x / scale, y: ev.cursor_y / scale };
                        }
                        return pendingRedockCoords;
                    })();
                    pendingRedockCoords = null;
                    if (coords) void tryRedockAtCursor(coords.x, coords.y);
                } else {
                    pendingRedockCoords = null;
                }
            },
        );

        // On Windows, Win32BeginMoveTask emits `floating-redock:hover-state` at
        // 50 ms with cursor_x/cursor_y (physical px). Both gates apply:
        // 1. Velocity: compute CSS-px/s from successive event positions; if
        //    > REDOCK_VELOCITY_PX_PER_S reset dwell clock and disarm.
        // 2. Dwell: arm only after the same target has been seen continuously for
        //    REDOCK_DWELL_MS ms (checked after the velocity gate passes).
        let stopHoverStateListener: (() => void) = () => {};
        if (isWindows()) {
            stopHoverStateListener = safeListenEvent<{
                target_label?: string | null;
                cursor_x?: number;
                cursor_y?: number;
            }>(
                "floating-redock:hover-state",
                (ev) => {
                    if (!dragging) return;
                    const newTarget = ev.target_label ?? null;
                    const now = performance.now();
                    // Velocity gate: cursor_x/cursor_y are physical px — divide by
                    // posScale() to get CSS px (1 DIP on non-HiDPI, 0.5 CSS on 2×).
                    if (typeof ev.cursor_x === "number" && typeof ev.cursor_y === "number") {
                        const scale = posScale();
                        const cssCurX = ev.cursor_x / scale;
                        const cssCurY = ev.cursor_y / scale;
                        if (dwellWinLastSampleAt > 0) {
                            const dt = now - dwellWinLastSampleAt;
                            const dx = cssCurX - dwellWinLastSampleX;
                            const dy = cssCurY - dwellWinLastSampleY;
                            const velocity = dt > 0 ? Math.sqrt(dx * dx + dy * dy) / (dt / 1000) : 0;
                            if (velocity > REDOCK_VELOCITY_PX_PER_S) {
                                // Moving fast — reset dwell clock and disarm.
                                // Preserve dwellCurrentHoverTarget (don't null it) so that
                                // if the cursor slows or stops over the same target the
                                // dwell timer can restart (codex P2 fix). Only the timer
                                // is cleared; the target identity is kept.
                                dwellCurrentHoverTarget = newTarget;
                                dwellHoverTargetFirstSeenAt = null;
                                hoverArmed = false;
                                invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
                                dwellWinLastSampleAt = now;
                                dwellWinLastSampleX = cssCurX;
                                dwellWinLastSampleY = cssCurY;
                                return;
                            }
                        }
                        dwellWinLastSampleAt = now;
                        dwellWinLastSampleX = cssCurX;
                        dwellWinLastSampleY = cssCurY;
                    }
                    // Dwell gate: arm only after seeing the same non-null target for
                    // REDOCK_DWELL_MS ms. Target change resets the clock.
                    if (newTarget !== dwellCurrentHoverTarget) {
                        dwellCurrentHoverTarget = newTarget;
                        dwellHoverTargetFirstSeenAt = newTarget !== null ? now : null;
                        const wasArmed = hoverArmed;
                        hoverArmed = false;
                        if (wasArmed) invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
                    } else if (newTarget !== null && dwellHoverTargetFirstSeenAt === null) {
                        // Velocity gate preserved the target but cleared the timer.
                        // Cursor has now slowed on the same target — restart dwell clock.
                        dwellHoverTargetFirstSeenAt = now;
                    } else if (
                        newTarget !== null &&
                        dwellHoverTargetFirstSeenAt !== null &&
                        now - dwellHoverTargetFirstSeenAt >= REDOCK_DWELL_MS
                    ) {
                        hoverArmed = true;
                    }
                },
            );
        }

        // macOS + Linux use the JS-driven drag (jsDrivenDrag), so the renderer
        // owns the gesture and keeps receiving mousemove. This handler drives
        // both the window position (jsDragSendPos) and the redock hover emit.
        // On Windows the renderer's mousemove is dark during the drag —
        // Win32BeginMoveTask emits hover host-side instead (§3.2 / §3.3 of spec).
        let unlistenMouseMove: (() => void) | null = null;
        if (!isWindows()) {
            const HOVER_THROTTLE_MS = 50;
            let lastHoverAt = 0;
            const onMouseMove = (e: MouseEvent) => {
                if (jsDrivenDrag) {
                    // Track latest coords even before dragging is armed — needed for
                    // catch-up in onMouseDown's get_window_position .then() callback.
                    jsDragLatestScreenX = e.screenX;
                    jsDragLatestScreenY = e.screenY;
                }
                if (!dragging) return;
                hasMoved = true;
                if (jsDrivenDrag) {
                    // JS-driven position update — compute delta from mousedown origin
                    // and call set_window_position with a one-in-flight + coalesce guard.
                    const scale = posScale();
                    jsDragSendPos(
                        jsDragMouseDownId,
                        jsDragInitWinX + Math.round((e.screenX - jsDragClickScreenX) * scale),
                        jsDragInitWinY + Math.round((e.screenY - jsDragClickScreenY) * scale),
                    );
                }
                const now = performance.now();
                // Velocity gate: measure cursor speed since the last sample.
                const dt = now - dwellLastMoveSampleAt;
                const dx = e.screenX - dwellLastMoveSampleX;
                const dy = e.screenY - dwellLastMoveSampleY;
                if (dwellLastMoveSampleAt > 0 && dt > 0) {
                    const velocity = Math.sqrt(dx * dx + dy * dy) / (dt / 1000);
                    if (velocity > REDOCK_VELOCITY_PX_PER_S) {
                        dwellSlowSince = null;
                        dwellLastArmedTarget = null;
                        dwellCurrentConfirmedAt = null;
                        if (hoverArmed || indicatorShowing) {
                            hoverArmed = false;
                            indicatorShowing = false;
                            invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
                        }
                        dwellLastMoveSampleAt = now;
                        dwellLastMoveSampleX = e.screenX;
                        dwellLastMoveSampleY = e.screenY;
                        return;
                    }
                }
                dwellLastMoveSampleAt = now;
                dwellLastMoveSampleX = e.screenX;
                dwellLastMoveSampleY = e.screenY;
                if (dwellSlowSince === null) dwellSlowSince = now;
                // Fire the hover IPC HOVER_THROTTLE_MS before the arm threshold so
                // the round-trip completes and the ghost is visible before onMouseUp
                // can fire — otherwise the user sees no ghost but the block docks.
                if (now - dwellSlowSince < REDOCK_DWELL_MS - HOVER_THROTTLE_MS) return;
                // Cursor has been slow for REDOCK_DWELL_MS - HOVER_THROTTLE_MS — throttle IPC calls.
                if (now - lastHoverAt < HOVER_THROTTLE_MS) return;
                lastHoverAt = now;
                const scale = posScale();
                const sourceLabel = windowLabel();
                if (!sourceLabel) return;
                const capturedSessionId = dragSessionId;
                invokeCommand<{ target_label?: string | null }>("update_floating_redock_hover", {
                    source_label: sourceLabel,
                    x: Math.round(e.screenX * scale),
                    y: Math.round(e.screenY * scale),
                }).then((res) => {
                    if (capturedSessionId !== dragSessionId) return;
                    const newTarget = res?.target_label ?? null;
                    if (newTarget !== dwellLastArmedTarget) {
                        // Target changed — disarm old target and restart both dwell
                        // clocks. dwellSlowSince reset prevents a slow transit across
                        // an intermediate window from pre-satisfying the dwell for the
                        // destination. dwellCurrentConfirmedAt records when this target
                        // was first confirmed so the mouseup fallback uses "180ms since
                        // first confirmation" (prevents desktop-transit false arm).
                        // Exception: null → target is the initial hover confirmation, not
                        // a transit. Keeping dwellSlowSince lets onMouseUp's condition-2
                        // arm fire even when the IPC was sent early and the user releases
                        // immediately after the ghost appears.
                        const prevTarget = dwellLastArmedTarget;
                        dwellLastArmedTarget = newTarget;
                        dwellCurrentHoverTarget = newTarget; // Phase 4b: mirror for ghost capture in onMouseUp (Windows sets this via event; macOS must set it here)
                        dwellCurrentConfirmedAt = newTarget !== null ? performance.now() : null;
                        if (prevTarget !== null) dwellSlowSince = null;
                        if (hoverArmed || indicatorShowing) {
                            hoverArmed = false;
                            indicatorShowing = false;
                            invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
                        }
                        // Backend has now broadcast the indicator for the new target.
                        // Mark it showing so the velocity gate can clear it on fast
                        // escape. hoverArmed stays false — full dwell still required.
                        indicatorShowing = newTarget !== null;
                    } else if (newTarget !== null) {
                        indicatorShowing = true;
                        hoverArmed = true;
                    }
                }).catch(() => {});
            };
            document.addEventListener("mousemove", onMouseMove);
            unlistenMouseMove = () => document.removeEventListener("mousemove", onMouseMove);
        }

        const tryRedockAtCursor = async (screenX: number, screenY: number) => {
            setRedockInProgress(true);
            try {
                await tryRedockAtCursorInner(screenX, screenY);
            } finally {
                setRedockInProgress(false);
            }
        };

        const tryRedockAtCursorInner = async (screenX: number, screenY: number) => {
            const ourLabel = windowLabel();
            if (!ourLabel || cleaned) return;
            // `mouseup.screenX/Y` are CSS px. Host coordinate space: physical px
            // on Windows (× DPR), DIP on macOS/Linux (× 1) — posScale().
            const scale = posScale();
            const px = Math.round(screenX * scale);
            const py = Math.round(screenY * scale);

            let target: { label: string | null; window_id: string | null };
            try {
                // exclude_label = our own floater's label. Without this,
                // the host's Z-order walk would return the floater itself
                // — it's at the cursor (the JS-driven drag follows the
                // cursor), so it's always topmost where the cursor is.
                target = await invokeCommand<{
                    label: string | null;
                    window_id: string | null;
                }>("resolve_window_at_cursor", {
                    x: px,
                    y: py,
                    exclude_label: ourLabel,
                });
            } catch (e) {
                console.error("[floating-pane] resolve_window_at_cursor failed", e);
                return;
            }
            if (cleaned) return;
            if (!target.label || !target.window_id) {
                // Cursor over desktop, external app, or our own floater
                // — leave floater at the dropped position.
                return;
            }

            // Resolve target's active tab via the WaveObj graph:
            // window → workspace.activetabid. Use the non-pinning
            // `reloadWaveObject` — `loadAndPinWaveObject` would bump
            // `refCount` with no matching unpin in this async flow,
            // leaking one ref on each of the target Window + Workspace
            // per successful redock so the cache cleanup never evicts
            // them.
            let targetWs: Workspace;
            try {
                const targetWindow = await WOS.reloadWaveObject<WaveWindow>(
                    WOS.makeORef("window", target.window_id),
                );
                targetWs = await WOS.reloadWaveObject<Workspace>(
                    WOS.makeORef("workspace", targetWindow.workspaceid),
                );
            } catch (e) {
                console.error(
                    "[floating-pane] failed to resolve target window's workspace",
                    e,
                );
                return;
            }
            if (cleaned) return;
            const targetTabId = targetWs.activetabid;
            const targetWsId = targetWs.oid;
            if (!targetTabId || !targetWsId) {
                console.warn(
                    "[floating-pane] target window has no active tab — skipping redock",
                );
                return;
            }

            // Source identifiers — the floater's only-tab + only-block.
            const sourceTabId = tabId();
            const sourceWs = ws();
            if (!sourceTabId || !sourceWs) return;
            const sourceWsId = sourceWs.oid;
            // Non-reactive read: `useWaveObjectValue` would register an
            // `onCleanup` against the current reactive owner, but we're inside
            // an async mouseup callback with no owner — the refCount would
            // never get decremented and we'd leak a Tab subscription per drop.
            const sourceTabObj = WOS.getObjectValue<Tab>(
                WOS.makeORef("tab", sourceTabId),
            );
            const sourceBlockId = sourceTabObj?.blockids?.[0];
            if (!sourceBlockId) {
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
                capturedGhostForDrop != null && capturedGhostForWindow === target.label
                    ? await capturedGhostForDrop
                    : {};
            capturedGhostForDrop = null;
            capturedGhostForWindow = null;

            try {
                await WorkspaceService.RedockFloatingPane(
                    sourceBlockId,
                    sourceTabId,
                    sourceWsId,
                    targetTabId,
                    targetWsId,
                    ghost.block_id ?? null,
                    ghost.dir ?? null,
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
        };

        // Both mousedown and mouseup are capture-phase so they always fire
        // before any child stopPropagation call can suppress them. mousedown
        // must be capture to pre-empt pragmatic-dnd's dragstart; mouseup must
        // be capture to ensure dragging is always reset (a bubble-phase mouseup
        // blocked by a child would silently leave dragging=true).
        document.addEventListener("mousedown", onMouseDown, true);
        document.addEventListener("mouseup", onMouseUp, true);

        onCleanup(() => {
            cleaned = true;
            document.removeEventListener("mousedown", onMouseDown, true);
            document.removeEventListener("mouseup", onMouseUp, true);
            stopCancelListener();
            stopEndedListener();
            stopHoverStateListener();
            unlistenMouseMove?.();
        });
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
