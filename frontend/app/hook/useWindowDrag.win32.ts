// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Windows-specific window drag hook.
// Tauri: data-tauri-drag-region is handled at the WebView/OS level.
// CEF: document-level mousedown checks for drag regions and triggers IPC.

import { detectHost, invokeCommand } from "@/app/platform/ipc";

let cefDragListenerInstalled = false;

function installCefDragListener() {
    if (cefDragListenerInstalled || detectHost() !== "cef") return;
    cefDragListenerInstalled = true;

    document.addEventListener("mousedown", (e: MouseEvent) => {
        if (e.button !== 0) return;

        // Walk up from the click target to find if we're in a drag region.
        // data-tauri-drag-region="true" means draggable (set on header, status bar).
        // data-tauri-drag-region="false" means NOT draggable (buttons, tabs, inputs).
        let el = e.target as HTMLElement | null;
        let inDragRegion = false;
        while (el) {
            const attr = el.getAttribute("data-tauri-drag-region");
            if (attr === "false") {
                // Explicitly opted out — don't drag
                return;
            }
            if (attr === "true" || attr === "") {
                inDragRegion = true;
                break;
            }
            el = el.parentElement;
        }

        if (inDragRegion) {
            e.preventDefault();
            invokeCommand("start_window_drag").catch(() => {});
        }
    }, true); // capture phase — fires before bubbling
}

export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    installCefDragListener();
    return { dragProps: { "data-tauri-drag-region": true } };
}
