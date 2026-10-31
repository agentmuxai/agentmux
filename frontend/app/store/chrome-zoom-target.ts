// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a Ctrl+wheel over `target` zooms chrome rather than its pane. Shared
 * by AppZoomHandler and AppAllPanesZoomHandler (app.tsx), which route a wheel
 * by this one hit-test, so which elements count as chrome has one place to
 * change (#3090). A leaf module so it can be tested without
 * app.tsx's imports.
 *
 * `data-zoom-scope` says which zoom an element is drawn at, and the nearest one wins: a pane that
 * follows chrome zoom as a whole (the Editor showing a PDF) marks its content `content`, which a
 * wheel over it zooms as the pane.
 */
export function isOverChrome(target: HTMLElement): boolean {
    const scope = target.closest("[data-zoom-scope]")?.getAttribute("data-zoom-scope");
    if (scope) return scope === "chrome";
    return (
        !!target.closest(".window-header") ||
        !!target.closest(".status-bar") ||
        !!target.closest(".block-frame-default-header")
    );
}
