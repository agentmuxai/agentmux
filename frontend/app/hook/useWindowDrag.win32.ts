// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Windows-specific window drag hook.
//
// Two implementations, chosen at install time:
//
//  - NATIVE (default) — SPEC_WINDOW_DRAG_NATIVE_MOVE_LOOP_2026_05_29.md.
//    Keep the title bar HTCLIENT (so right-click / contextmenu still fire),
//    detect drag intent in JS, and on a 4px threshold hand the move to the OS
//    via ONE fire-and-forget `start_window_drag` IPC. The host runs
//    `ReleaseCapture()` + `SendMessageW(WM_NCLBUTTONDOWN, HTCAPTION)`, so the
//    OS modal move loop tracks the cursor with ZERO further IPC — matching
//    VS Code / Electron smoothness. This mirrors useWindowDrag.linux.ts.
//
//  - LEGACY JS (fallback) — JS-driven per-mousemove `set_window_position`.
//    Smooth-but-laggy: the window position is gated on an IPC round-trip per
//    move, so it lags the cursor under any host jitter. Kept behind a runtime
//    opt-out while the native path is validated, because of a historical note
//    that the native `WM_NCLBUTTONDOWN` path "loses mouse state" (this hook is
//    the spike that verifies whether that still reproduces — the previous
//    attempt awaited an IPC before sending; the native path here never awaits).
//
// Toggle: native is ON by default. To force the legacy JS path, set
//   localStorage['agentmux.win32NativeDrag'] = '0'
// in DevTools and reload the window.

import { detectHost, invokeCommand } from "@/app/platform/ipc";

let cefDragListenerInstalled = false;

// Threshold in CSS pixels before we treat a press+move as a window drag.
// Below this the press is a normal click (buttons inside a drag region still
// work). 4px matches Chrome's default drag threshold (and the Linux hook).
const DRAG_THRESHOLD_PX = 4;

/// Native OS move loop is the default. Set localStorage
/// 'agentmux.win32NativeDrag' = '0' (then reload) to fall back to the legacy
/// JS-driven drag. Wrapped in try/catch because localStorage can throw in
/// locked-down contexts.
function useNativeDrag(): boolean {
    try {
        return localStorage.getItem("agentmux.win32NativeDrag") !== "0";
    } catch {
        return true;
    }
}

function isInDragRegion(target: HTMLElement | null): boolean {
    let el = target;
    while (el) {
        const attr = el.getAttribute("data-drag-region");
        if (attr === "false") return false;
        if (attr === "true" || attr === "") return true;
        el = el.parentElement;
    }
    return false;
}

/// Identify which top-level window we're running in so the host can route
/// drag/maximize IPC to the right HWND. The renderer URL carries
/// `windowLabel=…` for every non-main window (the launcher doesn't add it for
/// `main`); falling back to `"main"` keeps the main window pointing at itself.
function ownWindowLabel(): string {
    try {
        return new URLSearchParams(window.location.search).get("windowLabel") || "main";
    } catch {
        return "main";
    }
}

// ── NATIVE path ──────────────────────────────────────────────────────────
// Mirrors useWindowDrag.linux.ts: arm on mousedown, hand the move to the OS
// on threshold crossing, let the OS modal loop run the rest. No per-move IPC,
// no DPI math, no race-guarding — the OS owns the move.
function installNativeDragListener() {
    let pressX = 0;
    let pressY = 0;
    let pressArmed = false; // mousedown seen, waiting for threshold-crossing motion
    let dragInitiated = false; // start_window_drag has been sent

    document.addEventListener(
        "mousedown",
        (e: MouseEvent) => {
            // Left button only; right/middle pass through to standard handling
            // (contextmenu etc) — the whole reason we keep the region HTCLIENT.
            if (e.button !== 0) return;
            if (!isInDragRegion(e.target as HTMLElement)) return;
            pressX = e.clientX;
            pressY = e.clientY;
            pressArmed = true;
            dragInitiated = false;
            // No preventDefault: a sub-threshold click must still reach child
            // handlers, and the button must stay "down" for the OS move loop.
        },
        true,
    );

    document.addEventListener(
        "mousemove",
        (e: MouseEvent) => {
            if (!pressArmed || dragInitiated) return;
            // Primary button released (possibly outside the webview) → disarm
            // so a stray hover after an interrupted press can't start a drag.
            if ((e.buttons & 1) === 0) {
                pressArmed = false;
                return;
            }
            const dx = Math.abs(e.clientX - pressX);
            const dy = Math.abs(e.clientY - pressY);
            if (dx < DRAG_THRESHOLD_PX && dy < DRAG_THRESHOLD_PX) return;
            // Threshold crossed — the button is still physically down here,
            // which is exactly what the WM_NCLBUTTONDOWN modal loop needs.
            // Fire ONE fire-and-forget IPC and stop tracking; the OS runs the
            // move until the user releases. NEVER await it on the input path.
            dragInitiated = true;
            pressArmed = false;
            invokeCommand("start_window_drag", { label: ownWindowLabel() }).catch(() => {
                dragInitiated = false;
            });
        },
        true,
    );

    document.addEventListener(
        "mouseup",
        () => {
            pressArmed = false;
            dragInitiated = false;
        },
        true,
    );

    document.addEventListener(
        "dblclick",
        (e: MouseEvent) => {
            if (e.button !== 0) return;
            if (!isInDragRegion(e.target as HTMLElement)) return;
            e.preventDefault();
            pressArmed = false;
            dragInitiated = false;
            invokeCommand("maximize_window", { label: ownWindowLabel() }).catch(() => {});
        },
        true,
    );
}

// ── LEGACY JS path (fallback) ──────────────────────────────────────────────
// JS-driven window move: track mouse delta, set absolute window position via
// IPC per mousemove. Kept verbatim (Codex PR #734 race-guarding intact) behind
// the localStorage opt-out. See the file header for why it lags.
function installJsDragListener() {
    // Per-mousedown sequence token. Each press increments `currentMouseDownId`;
    // the async `get_window_position` handler captures the value at press time
    // and bails if it doesn't still match when the promise resolves.
    // (Codex P2 PR #734 round 2.)
    let currentMouseDownId = 0;
    let dragging = false;
    let clickScreenX = 0;
    let clickScreenY = 0;
    let initWinX = 0;
    let initWinY = 0;
    // Latest cursor position seen during a press, even before `dragging` is
    // armed, so the get_window_position resolution can catch up (PR #734 r4).
    let latestScreenX = 0;
    let latestScreenY = 0;

    document.addEventListener(
        "mousedown",
        async (e: MouseEvent) => {
            if (e.button !== 0) return;
            if (!isInDragRegion(e.target as HTMLElement)) return;
            e.preventDefault();
            currentMouseDownId += 1;
            const myId = currentMouseDownId;
            clickScreenX = e.screenX;
            clickScreenY = e.screenY;
            latestScreenX = e.screenX;
            latestScreenY = e.screenY;
            try {
                const pos = await invokeCommand<{ x: number; y: number }>("get_window_position", {
                    label: ownWindowLabel(),
                });
                // Race guard: bail if a mouseup or a newer mousedown happened
                // during the IPC round-trip.
                if (myId !== currentMouseDownId) return;
                initWinX = pos.x;
                initWinY = pos.y;
                dragging = true;
                // Catch-up: if the cursor moved during the IPC, fire one
                // set_window_position immediately against the latest position.
                // DPI: e.screenX is CSS px; initWinX is Win32 physical px —
                // multiply the CSS delta by devicePixelRatio + round.
                // See docs/specs/SPEC_WINDOW_DRAG_DPI_FIX_2026-05-13.md.
                if (latestScreenX !== clickScreenX || latestScreenY !== clickScreenY) {
                    const dpr = window.devicePixelRatio || 1;
                    const tx = initWinX + Math.round((latestScreenX - clickScreenX) * dpr);
                    const ty = initWinY + Math.round((latestScreenY - clickScreenY) * dpr);
                    sendPos(tx, ty);
                }
            } catch {
                // host unavailable — abort drag
            }
        },
        true,
    );

    // One-in-flight + coalesce: at most one set_window_position in flight; if
    // more mousemoves arrive, stash the latest and fire on completion. Older
    // positions are dropped (stale). (Codex P2 PR #734 round 3.)
    let setPosInFlight = false;
    let pendingPos: { x: number; y: number } | null = null;
    const sendPos = (x: number, y: number): void => {
        if (setPosInFlight) {
            pendingPos = { x, y };
            return;
        }
        setPosInFlight = true;
        invokeCommand("set_window_position", { x, y, label: ownWindowLabel() })
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
    document.addEventListener("mousemove", (e: MouseEvent) => {
        latestScreenX = e.screenX;
        latestScreenY = e.screenY;
        if (!dragging) return;
        // CSS-pixel delta * devicePixelRatio = physical delta. Re-read DPR
        // every move so a mid-drag monitor crossing picks up the new scale.
        const dpr = window.devicePixelRatio || 1;
        const tx = initWinX + Math.round((e.screenX - clickScreenX) * dpr);
        const ty = initWinY + Math.round((e.screenY - clickScreenY) * dpr);
        sendPos(tx, ty);
    });

    document.addEventListener("mouseup", () => {
        currentMouseDownId += 1;
        dragging = false;
        // Do NOT clear pendingPos — let the in-flight set_window_position drain
        // to the cursor's release position. (PR #734 round 4.)
    });

    document.addEventListener(
        "dblclick",
        (e: MouseEvent) => {
            if (e.button !== 0) return;
            if (!isInDragRegion(e.target as HTMLElement)) return;
            e.preventDefault();
            dragging = false;
            invokeCommand("maximize_window").catch(() => {});
        },
        true,
    );
}

function installCefDragListener() {
    if (cefDragListenerInstalled || detectHost() !== "cef") return;
    cefDragListenerInstalled = true;
    if (useNativeDrag()) {
        console.info("[window-drag] win32: NATIVE OS move loop (start_window_drag)");
        installNativeDragListener();
    } else {
        console.info("[window-drag] win32: legacy JS-driven drag (set_window_position)");
        installJsDragListener();
    }
}

export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    installCefDragListener();
    return { dragProps: { "data-drag-region": true } };
}
