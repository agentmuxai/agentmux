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

    // `mouseDownActive` is set synchronously in mousedown and cleared
    // in mouseup. `dragging` is set after the async `get_window_position`
    // resolves. The mousedown→mouseup race (codex P2 PR #734): if the
    // user clicks and releases quickly (or the IPC round-trip is slow),
    // `mouseup` fires while `get_window_position` is still in flight;
    // when the promise resolves we'd otherwise arm drag AFTER release
    // and the next mousemove (a stray pointer move) would teleport
    // the window. Guard by checking `mouseDownActive` at resolution.
    let mouseDownActive = false;
    let dragging = false;
    let clickScreenX = 0;
    let clickScreenY = 0;
    let initWinX = 0;
    let initWinY = 0;

    document.addEventListener("mousedown", async (e: MouseEvent) => {
        if (e.button !== 0) return;
        if (!isInDragRegion(e.target as HTMLElement)) return;
        e.preventDefault();
        mouseDownActive = true;
        try {
            const pos = await invokeCommand<{ x: number; y: number }>("get_window_position");
            // Race guard: bail if the user has already released the
            // mouse during the IPC round-trip.
            if (!mouseDownActive) return;
            clickScreenX = e.screenX;
            clickScreenY = e.screenY;
            initWinX = pos.x;
            initWinY = pos.y;
            dragging = true;
        } catch {
            // host unavailable — abort drag
        }
    }, true);

    document.addEventListener("mousemove", (e: MouseEvent) => {
        if (!dragging) return;
        const tx = initWinX + (e.screenX - clickScreenX);
        const ty = initWinY + (e.screenY - clickScreenY);
        invokeCommand("set_window_position", { x: tx, y: ty }).catch(() => {});
    });

    document.addEventListener("mouseup", () => {
        mouseDownActive = false;
        dragging = false;
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
