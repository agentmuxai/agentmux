// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// macOS-specific window drag hook — JS-driven, so drag and right-click
// coexist on the same surface.
//
// Why NOT `-webkit-app-region: drag`: Chromium architecturally suppresses
// ALL events (including `contextmenu`) on app-region drag elements before
// they reach the renderer, so a native drag region and right-click are
// mutually exclusive on the same element. Instead the header stays
// HTCLIENT (no app-region; see window-header.darwin.scss) — right-click
// fires everywhere — and we detect drag in JS, **left-button only**, so
// right/middle-click pass straight through to `contextmenu`.
//
// On left-mousedown in a `data-drag-region` element + threshold-crossing
// motion, we send ONE `start_window_drag` IPC. The host turns it into a
// native AppKit drag (`[NSWindow performWindowDragWithEvent:]`, see
// agentmux-cef/src/ui_tasks.rs) — the window server moves the window, so
// there's no per-frame IPC. Mirrors the Linux model (which routes the same
// IPC to `CefWindow::BeginWindowDrag`); macOS stays on stock libcef.

import { detectHost, invokeCommand } from "@/app/platform/ipc";

let cefDragListenerInstalled = false;

// Each CEF window's frontend carries its own `?windowLabel=…`. The host
// IPC handlers default to "main" when the label is missing, so single-
// window builds still work.
function currentWindowLabel(): string {
    try {
        return new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
    } catch {
        return "main";
    }
}

// Threshold in CSS pixels before a press becomes a drag. Below this a
// press is a normal click (e.g. for buttons that sit inside a drag-region
// container). 4px matches Chrome's default drag threshold.
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
    let pressArmed = false; // mousedown registered, waiting for threshold motion
    let dragInitiated = false; // start_window_drag IPC has been sent

    document.addEventListener(
        "mousedown",
        (e: MouseEvent) => {
            // Left button only; right/middle-click pass through to the
            // renderer's normal handling (contextmenu etc.).
            if (e.button !== 0) return;
            if (!isInDragRegion(e.target as HTMLElement)) return;
            pressX = e.clientX;
            pressY = e.clientY;
            pressArmed = true;
            dragInitiated = false;
            // No preventDefault — the click must still work for any child
            // handler if it never reaches the drag threshold.
        },
        true,
    );

    document.addEventListener(
        "mousemove",
        (e: MouseEvent) => {
            if (!pressArmed || dragInitiated) return;
            // Primary button released outside the webview (the renderer may
            // not deliver a mouseup in that case) — disarm so we don't fire
            // on a stray hover after an interrupted press. e.buttons bit 0 =
            // primary button currently held.
            if ((e.buttons & 1) === 0) {
                pressArmed = false;
                return;
            }
            const dx = Math.abs(e.clientX - pressX);
            const dy = Math.abs(e.clientY - pressY);
            if (dx < DRAG_THRESHOLD_PX && dy < DRAG_THRESHOLD_PX) return;
            // Threshold crossed — hand off to the native AppKit drag. ONE
            // IPC; the window server takes over until the mouse is released.
            dragInitiated = true;
            pressArmed = false;
            invokeCommand("start_window_drag", { label: currentWindowLabel() }).catch(() => {
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

    // Double-click on the drag region toggles maximize (macOS zoom).
    document.addEventListener(
        "dblclick",
        (e: MouseEvent) => {
            if (e.button !== 0) return;
            if (!isInDragRegion(e.target as HTMLElement)) return;
            e.preventDefault();
            pressArmed = false;
            dragInitiated = false;
            invokeCommand("maximize_window", { label: currentWindowLabel() }).catch(() => {});
        },
        true,
    );
}

export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    installCefDragListener();
    // The marker drives the JS listener, installed only for CEF. On non-CEF
    // hosts emit no attribute so the element stays strictly HTCLIENT.
    if (detectHost() !== "cef") return { dragProps: {} };
    // Tabs/buttons inside the header set data-drag-region="false" to opt out
    // of the drag listener individually.
    return { dragProps: { "data-drag-region": true } };
}
