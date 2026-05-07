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

    document.addEventListener("mousedown", async (e: MouseEvent) => {
        if (e.button !== 0) return;
        if (!isInDragRegion(e.target as HTMLElement)) return;
        e.preventDefault();
        currentMouseDownId += 1;
        const myId = currentMouseDownId;
        try {
            const pos = await invokeCommand<{ x: number; y: number }>("get_window_position");
            // Race guard: bail if a mouseup or a newer mousedown has
            // happened during the IPC round-trip.
            if (myId !== currentMouseDownId) return;
            clickScreenX = e.screenX;
            clickScreenY = e.screenY;
            initWinX = pos.x;
            initWinY = pos.y;
            dragging = true;
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
        if (!dragging) return;
        const tx = initWinX + (e.screenX - clickScreenX);
        const ty = initWinY + (e.screenY - clickScreenY);
        sendPos(tx, ty);
    });

    document.addEventListener("mouseup", () => {
        // Invalidate any in-flight mousedown handler — incrementing
        // the id ensures their `myId !== currentMouseDownId` check
        // fires when they resolve.
        currentMouseDownId += 1;
        dragging = false;
        // Drop any queued post-drag position so a stray pendingPos
        // doesn't fire after release.
        pendingPos = null;
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
