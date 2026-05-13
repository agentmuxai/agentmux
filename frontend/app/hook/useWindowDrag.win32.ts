// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Windows-specific window drag hook.
// Tauri: data-drag-region is handled at the WebView/OS level.
// CEF: JS-driven window move — track mouse delta, set window position via IPC.
// WM_NCLBUTTONDOWN doesn't work because the async IPC roundtrip loses mouse state.

import { detectHost, invokeCommand } from "@/app/platform/ipc";

let cefDragListenerInstalled = false;

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

function installCefDragListener() {
    if (cefDragListenerInstalled || detectHost() !== "cef") return;
    cefDragListenerInstalled = true;

    // Per-mousedown sequence token. Each press increments
    // `currentMouseDownId`; the async `get_window_position` handler
    // captures the value at press time and bails if it doesn't still
    // match when the promise resolves.
    //
    // Without the sequence token, a rapid press → release → press
    // sequence with a slow IPC could let the OLDER request arm drag
    // using the older click coords (a shared boolean would already
    // be true again from the newer press). Codex P2 PR #734 round 2.
    let currentMouseDownId = 0;
    let dragging = false;
    let clickScreenX = 0;
    let clickScreenY = 0;
    let initWinX = 0;
    let initWinY = 0;
    // Track the latest cursor position seen during a press, even
    // before `dragging` is armed. When the get_window_position IPC
    // resolves we can immediately catch up with one set_window_position
    // call against the latest cursor — otherwise mousemoves during
    // the initial round-trip are silently dropped (codex P2 PR #734
    // round 4).
    let latestScreenX = 0;
    let latestScreenY = 0;

    document.addEventListener("mousedown", async (e: MouseEvent) => {
        if (e.button !== 0) return;
        if (!isInDragRegion(e.target as HTMLElement)) return;
        e.preventDefault();
        currentMouseDownId += 1;
        const myId = currentMouseDownId;
        // Capture press coords synchronously — used as the baseline
        // for both the live drag math and the catch-up move below.
        clickScreenX = e.screenX;
        clickScreenY = e.screenY;
        latestScreenX = e.screenX;
        latestScreenY = e.screenY;
        try {
            const pos = await invokeCommand<{ x: number; y: number }>("get_window_position");
            // Race guard: bail if a mouseup or a newer mousedown has
            // happened during the IPC round-trip.
            if (myId !== currentMouseDownId) return;
            initWinX = pos.x;
            initWinY = pos.y;
            dragging = true;
            // Catch-up: if the cursor moved during the IPC, fire one
            // set_window_position immediately against the latest known
            // position so we don't lose the first few pixels of motion.
            //
            // DPI scaling: `e.screenX` exposes CSS pixels (Blink divides
            // physical by combined browser-zoom under use-zoom-for-dsf,
            // default on Windows since Chrome 54); `initWinX` from
            // `get_window_position` and `set_window_position` use Win32
            // physical pixels in this PMv2 process. Multiply the CSS-
            // pixel delta by devicePixelRatio + round before adding to
            // the physical baseline. Without this, Win11's default 125%
            // scale makes the window lag the cursor by ~20%.
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
    }, true);

    // One-in-flight + coalesce. Native mousemove fires at ~120Hz; the
    // IPC round-trip can be slower under load. If two requests overlap
    // and the older one resolves AFTER the newer (e.g. transient host
    // jitter), the window snaps backward to the older absolute position.
    // Codex P2 PR #734 round 3.
    //
    // Strategy: at most one set_window_position in flight. If more
    // mousemoves arrive while in flight, stash the latest pending
    // position; on completion, fire that. Older positions are dropped
    // — they're stale by definition. Keeps the window tracking the
    // most recent cursor without ordering hazards.
    let setPosInFlight = false;
    let pendingPos: { x: number; y: number } | null = null;
    const sendPos = (x: number, y: number): void => {
        if (setPosInFlight) {
            pendingPos = { x, y };
            return;
        }
        setPosInFlight = true;
        invokeCommand("set_window_position", { x, y })
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
        // Track latest cursor position even before `dragging` is armed
        // so the catch-up at IPC resolution can use the most recent
        // value (otherwise initial mousemoves during the round-trip
        // are dropped).
        latestScreenX = e.screenX;
        latestScreenY = e.screenY;
        if (!dragging) return;
        // DPI scaling: CSS-pixel delta * devicePixelRatio = physical
        // delta, added to the physical-pixel baseline from
        // `get_window_position`. Re-read DPR every move so a mid-drag
        // monitor crossing (with different scale) picks up the new
        // value automatically. Spec:
        // docs/specs/SPEC_WINDOW_DRAG_DPI_FIX_2026-05-13.md §4.1-4.2.
        const dpr = window.devicePixelRatio || 1;
        const tx = initWinX + Math.round((e.screenX - clickScreenX) * dpr);
        const ty = initWinY + Math.round((e.screenY - clickScreenY) * dpr);
        sendPos(tx, ty);
    });

    document.addEventListener("mouseup", () => {
        // Invalidate any in-flight mousedown handler — incrementing
        // the id ensures their `myId !== currentMouseDownId` check
        // fires when they resolve.
        currentMouseDownId += 1;
        dragging = false;
        // Do NOT clear pendingPos — that would discard the FINAL
        // queued position (the cursor's location at release time)
        // and leave the window stranded at the previous in-flight
        // position. Let the in-flight set_window_position complete
        // naturally; its `.finally` will drain pendingPos to its
        // correct end state. (codex P2 PR #734 round 4.)
    });

    document.addEventListener("dblclick", (e: MouseEvent) => {
        if (e.button !== 0) return;
        if (!isInDragRegion(e.target as HTMLElement)) return;
        e.preventDefault();
        dragging = false;
        invokeCommand("maximize_window").catch(() => {});
    }, true);
}

export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    installCefDragListener();
    return { dragProps: { "data-drag-region": true } };
}
