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

import { createRedockArming } from "./redock-arming";
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
        // transits over another window at speed, and — since
        // SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md — when the user is
        // merely repositioning the floater above the main window rather than
        // asking to dock it. Both platform paths feed the SAME state machine
        // (redock-arming.ts) so they cannot drift; the fourteen hand-rolled
        // dwell/velocity variables that used to live here, and the
        // events-went-quiet fallbacks they supported, are what that module
        // replaces.
        const arming = createRedockArming();
        // Captured by whichever of DOM mouseup / window_drag_ended observes the
        // release first (their relative order is not guaranteed on Windows —
        // separate CEF IPC channels). The other reads this instead of
        // re-deriving, and `consumeArmedAtRelease` nulls it so the second
        // arrival cannot fire a redock on the already-redocked block.
        let armedAtRelease: boolean | null = null;
        const consumeArmedAtRelease = (): boolean => {
            const armed = armedAtRelease ?? arming.isArmed();
            armedAtRelease = null;
            arming.reset();
            return armed;
        };
        // Assigned by the non-Windows branch below (Windows gets its heartbeat
        // from the host's drag tick instead, so these stay no-ops there).
        let startHoverHeartbeat: () => void = () => {};
        let stopHoverHeartbeat: () => void = () => {};
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
                arming.reset();
                armedAtRelease = null;
                startHoverHeartbeat();
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
            arming.reset();
            armedAtRelease = null;
            dragSessionId += 1;
        };

        const onMouseUp = (e: MouseEvent) => {
            // Invalidate any in-flight JS-driven get_window_position IPC so a race
            // where mouseup fires before the promise resolves doesn't arm dragging.
            if (jsDrivenDrag) jsDragMouseDownId += 1;
            if (!dragging) return;
            dragging = false;
            stopHoverHeartbeat();
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
            const preGhostWindow = arming.target();
            capturedGhostForWindow = preGhostWindow;
            capturedGhostForDrop = preGhostWindow
                ? invokeCommand<{ block_id?: string; dir?: number }>(
                      "get_floating_redock_target",
                      { window_label: preGhostWindow },
                  ).catch(() => ({}))
                : Promise.resolve({});
            invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
            if (isWindows()) {
                // On Windows the redock itself is committed by window_drag_ended
                // (it carries host cursor coords, so it doesn't have to trust the
                // DOM's). The two events travel separate CEF IPC channels and
                // their relative order is not guaranteed, so record the arm
                // decision here and let whichever handler runs second read it
                // rather than re-deriving from state the first one may have
                // already torn down.
                armedAtRelease = arming.isArmed();
                pendingRedockCoords = { x: e.screenX, y: e.screenY };
            } else if (hasMoved) {
                // Non-Windows (macOS + Linux): the JS-driven path keeps the
                // renderer receiving mousemove+mouseup normally, so the redock
                // commits here. Consuming resets the arming state, which is what
                // stops a late window_drag_ended (the Linux race) from firing a
                // second tryRedockAtCursor on the already-redocked block.
                //
                // Invalidate in-flight IPC .then() as well, so a stale hover
                // response cannot re-arm after this release has been consumed
                // (reagent P1 on #1249's follow-up).
                dragSessionId += 1;
                if (consumeArmedAtRelease()) {
                    void tryRedockAtCursor(e.screenX, e.screenY, preGhostWindow);
                }
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
                    stopHoverHeartbeat();
                    hasMoved = false;
                    pendingRedockCoords = null;
                    arming.reset();
                    armedAtRelease = null;
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
                stopHoverHeartbeat();
                // Phase 4b — pre-capture ghost if onMouseUp hasn't already done so
                // (window_drag_ended can arrive before DOM mouseup on Windows — separate
                // CEF IPC channels). Guards against double-consume:
                // get_floating_redock_target removes the entry atomically, so the
                // second caller would get {} anyway, but skip the IPC entirely.
                //
                // MUST run before consumeArmedAtRelease() below, which resets the
                // arming state and with it arming.target().
                if (!capturedGhostForDrop) {
                    const preGhostWindow = arming.target();
                    capturedGhostForWindow = preGhostWindow;
                    capturedGhostForDrop = preGhostWindow
                        ? invokeCommand<{ block_id?: string; dir?: number }>(
                              "get_floating_redock_target",
                              { window_label: preGhostWindow },
                          ).catch(() => ({}))
                        : Promise.resolve({});
                }
                // Whichever of DOM mouseup / this event ran first recorded the
                // arm decision; this consumes it (and resets arming) so the two
                // orderings agree and neither can redock twice.
                const armedAtEnd = consumeArmedAtRelease();
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
                    if (coords) void tryRedockAtCursor(coords.x, coords.y, capturedGhostForWindow);
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
                    // cursor_x/cursor_y are physical px on Windows — divide by
                    // posScale() to get the CSS px the arming module expects
                    // (1 DIP on non-HiDPI, 0.5 CSS on 2×). The host emits these
                    // both from WM_MOUSEMOVE and, since the dwell spec, from the
                    // 100ms drag tick — so samples keep arriving while the
                    // cursor is stationary and dwell is measured, not inferred.
                    const scale = posScale();
                    const { clearIndicator } = arming.sample({
                        target: ev.target_label ?? null,
                        // Absent on the target-only teardown broadcast; the
                        // module reuses its last position rather than reading
                        // the gap as a jump from the origin.
                        x: typeof ev.cursor_x === "number" ? ev.cursor_x / scale : undefined,
                        y: typeof ev.cursor_y === "number" ? ev.cursor_y / scale : undefined,
                        t: performance.now(),
                    });
                    if (clearIndicator) {
                        invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
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
                const sourceLabel = windowLabel();
                if (!sourceLabel) return;
                const scale = posScale();
                const capturedSessionId = dragSessionId;
                // Sampled at REQUEST time so the velocity the module sees is the
                // cursor's, not the IPC round-trip's.
                const sampleX = screenX;
                const sampleY = screenY;
                const sampleT = now;
                invokeCommand<{ target_label?: string | null }>("update_floating_redock_hover", {
                    source_label: sourceLabel,
                    x: Math.round(screenX * scale),
                    y: Math.round(screenY * scale),
                }).then((res) => {
                    if (capturedSessionId !== dragSessionId) return;
                    if (sampleT < lastAppliedSampleT) return;
                    lastAppliedSampleT = sampleT;
                    const { clearIndicator } = arming.sample({
                        target: res?.target_label ?? null,
                        x: sampleX,
                        y: sampleY,
                        t: sampleT,
                    });
                    if (clearIndicator) {
                        invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
                    }
                }).catch(() => {});
            };

            // The renderer stops receiving mousemove the instant the cursor
            // holds still, so dwell fed from mousemove alone cannot advance
            // during exactly the gesture it measures. This is the JS
            // counterpart of the host's DRAG_TICK heartbeat on Windows
            // (agentmux-cef/src/ui_tasks/drag.rs) — same cadence, same reason.
            // SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md §5.1.
            const HOVER_HEARTBEAT_MS = 100;
            let heartbeatTimer: ReturnType<typeof setInterval> | null = null;
            startHoverHeartbeat = () => {
                if (heartbeatTimer !== null) return;
                heartbeatTimer = setInterval(() => {
                    // hasMoved gates the redock in onMouseUp, so emitting before the
                    // drag has moved would paint a ghost promising a dock that
                    // cannot happen — a press-and-hold on the header would arm
                    // the indicator after the dwell and then do nothing on
                    // release (codex P2 on #3124). The host heartbeat is gated
                    // on its own `moves > 0` for the same reason.
                    if (!dragging || !hasMoved) return;
                    emitHover(jsDragLatestScreenX, jsDragLatestScreenY);
                }, HOVER_HEARTBEAT_MS);
            };
            stopHoverHeartbeat = () => {
                if (heartbeatTimer !== null) {
                    clearInterval(heartbeatTimer);
                    heartbeatTimer = null;
                }
            };

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
                emitHover(e.screenX, e.screenY);
            };
            document.addEventListener("mousemove", onMouseMove);
            unlistenMouseMove = () => document.removeEventListener("mousemove", onMouseMove);
        }

        const tryRedockAtCursor = async (screenX: number, screenY: number, expectedTarget: string | null) => {
            setRedockInProgress(true);
            try {
                await tryRedockAtCursorInner(screenX, screenY, expectedTarget);
            } finally {
                setRedockInProgress(false);
            }
        };

        const tryRedockAtCursorInner = async (screenX: number, screenY: number, expectedTarget: string | null) => {
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
            stopHoverHeartbeat();
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
