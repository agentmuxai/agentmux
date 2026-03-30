// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Windows-specific window drag hook.
// Tauri: data-tauri-drag-region is handled at the WebView/OS level.
// CEF: JS-driven window move — track mouse delta, set window position via IPC.
// WM_NCLBUTTONDOWN doesn't work because the async IPC roundtrip loses mouse state.

import { detectHost, invokeCommand } from "@/app/platform/ipc";

let cefDragListenerInstalled = false;

function isInDragRegion(target: HTMLElement | null): boolean {
    let el = target;
    while (el) {
        const attr = el.getAttribute("data-tauri-drag-region");
        if (attr === "false") return false;
        if (attr === "true" || attr === "") return true;
        el = el.parentElement;
    }
    return false;
}

function installCefDragListener() {
    if (cefDragListenerInstalled || detectHost() !== "cef") return;
    cefDragListenerInstalled = true;

    let dragging = false;
    let startScreenX = 0;
    let startScreenY = 0;

    document.addEventListener("mousedown", (e: MouseEvent) => {
        if (e.button !== 0) return;
        if (!isInDragRegion(e.target as HTMLElement)) return;

        dragging = true;
        startScreenX = e.screenX;
        startScreenY = e.screenY;
        e.preventDefault();

        // Get initial window position
        invokeCommand<{ x: number; y: number }>("get_window_position").catch(() => {
            dragging = false;
        });
    }, true);

    document.addEventListener("mousemove", (e: MouseEvent) => {
        if (!dragging) return;
        const dx = e.screenX - startScreenX;
        const dy = e.screenY - startScreenY;
        if (dx === 0 && dy === 0) return;
        startScreenX = e.screenX;
        startScreenY = e.screenY;
        invokeCommand("move_window_by", { dx, dy }).catch(() => {});
    });

    document.addEventListener("mouseup", () => {
        dragging = false;
    });
}

export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    installCefDragListener();
    return { dragProps: { "data-tauri-drag-region": true } };
}
