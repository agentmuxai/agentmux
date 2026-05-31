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
 *  - window drag is **JS-driven**, installed by the `onMount` below:
 *    a targeted document mousedown listener scoped to
 *    `[data-role="block-header"]` (and skipping interactive elements
 *    via `target.closest('button, a, input, ...')`) drives
 *    `get/set_window_position` IPC on Windows and macOS. On Linux
 *    (Wayland forbids client-driven top-level repositioning) it instead
 *    fires a single `start_window_drag` IPC that hands the drag to the
 *    compositor — same path as the main window's `useWindowDrag.linux`.
 *    `preventDefault` on mousedown blocks the HTML5 dragstart
 *    pragmatic-dnd would otherwise have used, suppressing a "double
 *    tear-off" regression. See
 *    `docs/analyses/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md`
 *    for why we use JS-driven drag rather than OS HTCAPTION.
 *
 * The frontend code path is otherwise identical to the docked case:
 * Block / view-model / RPC subscriptions all behave the same.
 */

import { invokeCommand } from "@/app/platform/ipc";
import { isLinux, isWindows } from "@/util/platformutil";
import { FLOATER_EDGE_RESIZE_BORDER } from "@/app/workspace/floater-resize";
import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { TabContent } from "@/app/tab/tabcontent";
import { WorkspaceService } from "@/store/services";
import { atoms, getApi } from "@/store/global";
import * as WOS from "@/store/wos";
import { Show, createEffect, createMemo, onCleanup, onMount, type JSX } from "solid-js";

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
    createEffect(() => {
        const tid = tabId();
        if (!tid) return;
        const [tab] = WOS.useWaveObjectValue<Tab>(WOS.makeORef("tab", tid));
        const t = tab();
        if (!t) return;
        const blockids = t.blockids ?? [];
        if (blockids.length > 0) {
            hadBlocks = true;
        } else if (hadBlocks) {
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

    // Pane-header drag — JS-driven window move, scoped to the standard
    // docked-pane header (`[data-role="block-header"]`). Mirrors the
    // main-window pattern in `frontend/app/hook/useWindowDrag.win32.ts`
    // (one-in-flight + coalesce, DPR scaling, catch-up move on IPC
    // resolution, per-mousedown sequence token) — minus the
    // `data-drag-region` traversal because we scope explicitly to the
    // pane header instead of opting in via attributes.
    //
    // `preventDefault()` on the qualifying mousedown is load-bearing:
    // it suppresses the HTML5 dragstart pragmatic-dnd would otherwise
    // use to initiate a pane tear-off (TileLayout.win32.tsx:443-471).
    // Without it, dragging a floater's header would tear the block off
    // into ANOTHER floating window — the "double tear-off" bug.
    //
    // See docs/analyses/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md.
    onMount(() => {
        const INTERACTIVE_SELECTOR =
            "button, a, input, select, textarea, [role='button']";
        const HEADER_SELECTOR = '[data-role="block-header"]';

        let currentMouseDownId = 0;
        let dragging = false;
        let clickScreenX = 0;
        let clickScreenY = 0;
        let initWinX = 0;
        let initWinY = 0;
        let latestScreenX = 0;
        let latestScreenY = 0;
        let setPosInFlight = false;
        let pendingPos: { x: number; y: number } | null = null;

        // Capture once per mount — windowLabel is fixed for the
        // floater's lifetime. Used to route `get/set_window_position`
        // IPC to OUR HWND (not whichever top-level the OS happens to
        // enumerate first in Z-order).
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
            let startRect = { x: 0, y: 0, w: 0, h: 0 }; // physical px

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
                    startRect = { x: r.x, y: r.y, w: r.width, h: r.height };
                    resizing = true;
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
                // screenX/Y are CSS px; the window rect is physical px.
                const dpr = window.devicePixelRatio || 1;
                const dx = Math.round((e.screenX - startX) * dpr);
                const dy = Math.round((e.screenY - startY) * dpr);
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

        const sendPos = (x: number, y: number): void => {
            if (setPosInFlight) {
                pendingPos = { x, y };
                return;
            }
            setPosInFlight = true;
            invokeCommand("set_window_position", { x, y, label })
                .catch(() => {})
                .finally(() => {
                    setPosInFlight = false;
                    if (pendingPos) {
                        const { x: nx, y: ny } = pendingPos;
                        pendingPos = null;
                        sendPos(nx, ny);
                    }
                });
        };

        const onMouseDown = async (e: MouseEvent) => {
            if (e.button !== 0) return;
            const target = e.target as HTMLElement | null;
            if (!target) return;
            // Must be inside the standard pane header
            if (!target.closest(HEADER_SELECTOR)) return;
            // Skip interactive controls within the header so close /
            // magnify / mic / endIconButton clicks reach their handlers
            if (target.closest(INTERACTIVE_SELECTOR)) return;

            // Block the HTML5 dragstart pragmatic-dnd would have used —
            // without this, the click tears the pane off into ANOTHER
            // floating window (the double-tear-off regression).
            e.preventDefault();

            // Wayland forbids client-driven top-level repositioning, so
            // the get/set_window_position polling below is a no-op. Hand
            // the drag to the compositor via the same IPC the main window
            // uses (see useWindowDrag.linux.ts).
            if (isLinux()) {
                invokeCommand("start_window_drag", { label }).catch(() => {});
                return;
            }

            currentMouseDownId += 1;
            const myId = currentMouseDownId;
            clickScreenX = e.screenX;
            clickScreenY = e.screenY;
            latestScreenX = e.screenX;
            latestScreenY = e.screenY;

            try {
                const pos = await invokeCommand<{ x: number; y: number }>(
                    "get_window_position",
                    { label },
                );
                // Race guard: bail if mouseup or a newer mousedown
                // happened during the IPC round-trip
                if (myId !== currentMouseDownId) return;
                initWinX = pos.x;
                initWinY = pos.y;
                dragging = true;
                // Catch-up: if the cursor moved during the IPC, fire
                // one set_window_position immediately so we don't lose
                // the first few pixels of motion
                if (
                    latestScreenX !== clickScreenX ||
                    latestScreenY !== clickScreenY
                ) {
                    const scale = posScale();
                    const tx =
                        initWinX +
                        Math.round((latestScreenX - clickScreenX) * scale);
                    const ty =
                        initWinY +
                        Math.round((latestScreenY - clickScreenY) * scale);
                    sendPos(tx, ty);
                }
            } catch {
                // host unavailable — abort drag silently
            }
        };

        // Throttled redock-hover push — drives the highlight overlay
        // on whichever agentmux window the cursor is currently over.
        // ~50 ms cadence is plenty for highlight tracking and keeps
        // IPC load minimal during fast drags.
        const HOVER_PUSH_THROTTLE_MS = 50;
        let lastHoverPushAt = 0;
        const pushRedockHover = (screenX: number, screenY: number) => {
            const now = performance.now();
            if (now - lastHoverPushAt < HOVER_PUSH_THROTTLE_MS) return;
            lastHoverPushAt = now;
            // Host coordinate space: physical px on Windows, DIP on macOS/Linux
            // (posScale). resolve_window_at_cursor / the hover hit-test compare
            // against the same space.
            const scale = posScale();
            const px = Math.round(screenX * scale);
            const py = Math.round(screenY * scale);
            const sourceLabel = windowLabel();
            if (!sourceLabel) return;
            invokeCommand("update_floating_redock_hover", {
                source_label: sourceLabel,
                x: px,
                y: py,
            }).catch(() => {});
        };

        const onMouseMove = (e: MouseEvent) => {
            // Track the latest cursor position even before `dragging`
            // is armed so the catch-up at IPC resolution can use the
            // most recent value
            latestScreenX = e.screenX;
            latestScreenY = e.screenY;
            if (!dragging) return;
            // CSS-px delta * posScale() = host-coordinate delta added to the
            // baseline from get_window_position. On Windows posScale() is the
            // devicePixelRatio (physical px); on macOS/Linux it's 1 (CEF Views
            // DIP). Re-read every move so a mid-drag monitor crossing on
            // Windows picks up the new DPR automatically.
            const scale = posScale();
            const tx =
                initWinX + Math.round((e.screenX - clickScreenX) * scale);
            const ty =
                initWinY + Math.round((e.screenY - clickScreenY) * scale);
            sendPos(tx, ty);
            pushRedockHover(e.screenX, e.screenY);
        };

        const onMouseUp = (e: MouseEvent) => {
            // Invalidate any in-flight mousedown handler — incrementing
            // the id ensures their `myId !== currentMouseDownId` check
            // fires when they resolve.
            currentMouseDownId += 1;
            const wasDragging = dragging;
            dragging = false;
            // Do NOT clear pendingPos — that would discard the FINAL
            // queued position (the cursor's location at release time).
            // Let the in-flight set_window_position complete; its
            // `.finally` drains pendingPos to the correct end state.

            // Floating-pane re-dock (Phase 4a MVP): if we just finished
            // a drag and the cursor landed over another agentmux window
            // in this process, fire RedockFloatingPane to move our
            // single block into that window's active tab. The source
            // workspace's tab.blockids becomes empty → the createEffect
            // above auto-closes this floater.
            //
            // Cross-instance / cross-version safety is free: the host's
            // `resolve_window_at_cursor` looks up HWND in this process's
            // `window_hwnds` map — another agentmux version's HWNDs
            // aren't in it, so cross-process drops silently no-op.
            if (wasDragging) {
                // Always clear the hover state on release — the highlight
                // overlay needs to disappear whether or not we ended up
                // over a target. Fire-and-forget; the redock attempt
                // below doesn't depend on this completing.
                invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
                void tryRedockAtCursor(e.screenX, e.screenY);
            }
        };

        const tryRedockAtCursor = async (screenX: number, screenY: number) => {
            const ourLabel = windowLabel();
            if (!ourLabel) return;
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

            try {
                await WorkspaceService.RedockFloatingPane(
                    sourceBlockId,
                    sourceTabId,
                    sourceWsId,
                    targetTabId,
                    targetWsId,
                );
                // After successful redock, source tab.blockids empties
                // → the auto-close watcher dismisses the floater.
            } catch (e) {
                console.error("[floating-pane] RedockFloatingPane failed", e);
            }
        };

        // Capture-phase listeners so we run BEFORE pragmatic-dnd's
        // bubble-phase mousedown handler — preventDefault here blocks
        // the HTML5 dragstart it would have triggered.
        document.addEventListener("mousedown", onMouseDown, true);
        document.addEventListener("mousemove", onMouseMove);
        document.addEventListener("mouseup", onMouseUp);

        onCleanup(() => {
            document.removeEventListener("mousedown", onMouseDown, true);
            document.removeEventListener("mousemove", onMouseMove);
            document.removeEventListener("mouseup", onMouseUp);
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
