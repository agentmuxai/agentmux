// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Linux-specific window drag hook.
//
// Why JS-driven drag instead of `-webkit-app-region: drag`:
// Chromium architecturally suppresses ALL events on drag regions before
// they enter the renderer (verified 2026-05-02 via document capture-phase
// listener: zero mousedown / contextmenu fires on drag elements). That
// makes drag and right-click contextmenu mutually exclusive on the same
// element when using `-webkit-app-region: drag`.
//
// Workaround: keep the header HTCLIENT (so contextmenu fires normally)
// and detect drag in JS. On mousedown + threshold-crossing motion, send
// `start_window_drag` IPC to the Rust host. The host calls into the new
// CefWindow::BeginWindowDrag() (CEF patch) which dispatches the
// compositor-driven move (xdg_toplevel.move on Wayland, _NET_WM_MOVERESIZE
// on X11). The compositor handles the rest of the drag until the user
// releases the mouse button.
//
// Required CEF patch: BeginWindowDrag added to CefWindow API (commit
// associated with the agentmux/7680-... branch).
// Required Rust IPC: `start_window_drag` -> ui_tasks::post_start_drag.

import { detectHost, invokeCommand } from "@/app/platform/ipc";

let cefDragListenerInstalled = false;

// Threshold in CSS pixels before we initiate a window drag. Below this
// the click is treated as a normal click (e.g. for buttons that happen
// to be inside a drag-region container). 4px matches Chrome's default
// drag threshold and Mutter's input gesture threshold.
const DRAG_THRESHOLD_PX = 4;

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

    let pressX = 0;
    let pressY = 0;
    let pressArmed = false; // mousedown registered, waiting for threshold-crossing motion
    let dragInitiated = false; // start_window_drag IPC has been sent

    document.addEventListener("mousedown", (e: MouseEvent) => {
        // Left button only; right-click and middle-click pass through to
        // standard renderer event handling (contextmenu etc).
        if (e.button !== 0) return;
        if (!isInDragRegion(e.target as HTMLElement)) return;
        pressX = e.clientX;
        pressY = e.clientY;
        pressArmed = true;
        dragInitiated = false;
        // No preventDefault — we want the click to still work for any
        // child handler if it doesn't reach the threshold.
    }, true);

    document.addEventListener("mousemove", (e: MouseEvent) => {
        if (!pressArmed || dragInitiated) return;
        const dx = Math.abs(e.clientX - pressX);
        const dy = Math.abs(e.clientY - pressY);
        if (dx < DRAG_THRESHOLD_PX && dy < DRAG_THRESHOLD_PX) return;
        // Threshold crossed — initiate native window drag. This sends
        // ONE IPC; the compositor takes over until mouseup. JS doesn't
        // track further motion (would race with Mutter anyway).
        dragInitiated = true;
        pressArmed = false;
        invokeCommand("start_window_drag").catch(() => {
            dragInitiated = false;
        });
    }, true);

    document.addEventListener("mouseup", () => {
        pressArmed = false;
        dragInitiated = false;
    }, true);

    // Double-click on drag region toggles maximize.
    document.addEventListener("dblclick", (e: MouseEvent) => {
        if (e.button !== 0) return;
        if (!isInDragRegion(e.target as HTMLElement)) return;
        e.preventDefault();
        pressArmed = false;
        dragInitiated = false;
        invokeCommand("maximize_window").catch(() => {});
    }, true);
}

export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    installCefDragListener();
    // The drag-region marker drives the JS-driven drag listener, which is
    // only installed for CEF (see installCefDragListener). On non-CEF
    // Linux hosts the marker has no listener attached, so emit no
    // attribute — keeps the element strictly HTCLIENT and avoids any
    // future hook from misinterpreting it.
    if (detectHost() !== "cef") return { dragProps: {} };
    // Tabs/buttons inside the header already have data-drag-region="false"
    // via tabbar.tsx etc., so they opt out individually.
    return { dragProps: { "data-drag-region": true } };
}
