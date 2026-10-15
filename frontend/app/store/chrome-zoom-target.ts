// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a Ctrl+wheel over `target` zooms chrome rather than its pane. Shared
 * by AppZoomHandler and AppAllPanesZoomHandler (app.tsx), which route a wheel
 * by this one hit-test, so which elements count as chrome has one place to
 * change (#3090). A leaf module so it can be tested without
 * app.tsx's imports.
 */
export function isOverChrome(target: HTMLElement): boolean {
    return (
        !!target.closest(".window-header") ||
        !!target.closest(".status-bar") ||
        !!target.closest(".block-frame-default-header") ||
        // Anything else drawn at chrome zoom: an open AnchoredPopover that
        // follows it (portaled, so not inside the bars above), or a browser
        // pane's nav bar.
        !!target.closest('[data-zoom-scope="chrome"]')
    );
}
