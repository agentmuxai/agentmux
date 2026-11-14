// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/util/app-api";

/**
 * Whether browser panes in this window are CEF Views overlays on Windows
 * (AGENTMUX_PANE_BACKEND=views; the host reports it as `paneBackend` in the
 * about details). What such a pane does while a DOM overlay is over it is
 * `hidesPaneUnderOverlay`.
 *
 * Never in a floating pane's window (`?windowLabel=floating-…`, a warm pool
 * window included): on Windows that is not a CEF Views window, so a pane in it
 * is a child window whatever the setting.
 */
export function isViewsPanesOnWindows(): boolean {
    if (!navigator.userAgent.includes("Windows")) return false;
    const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "";
    return !label.startsWith("floating-") && getApi().getAboutModalDetails?.()?.paneBackend === "views";
}

/** The host found its runtime has no overlay clip after telling this page that it has (`paneClip`). */
let clipUnavailable = false;
let watching = false;

/**
 * Listen, once per window, for the host saying that its runtime cannot cut a pane after all. The host
 * can only know when a pane exists, which is after the about details were read, and says so each time
 * a pane is made in this window. A browser view calls this as it mounts, before its pane is made.
 */
export function watchPaneClip(): void {
    if (watching) return;
    watching = true;
    void getApi().listen?.("pane-clip-unavailable", () => {
        clipUnavailable = true;
    });
}

/**
 * Whether the host hides a browser pane whole while a DOM overlay (a menu, a
 * tooltip, a modal) is over it, so the page shows a frozen frame of it instead
 * (use-freeze-frame.ts). Only a Views pane on Linux or Windows whose host does
 * not cut the pane around the overlay: `paneClip` in the about details is off
 * (AGENTMUX_PANE_CLIP=off), or the host said since that its runtime has no clip
 * (`watchPaneClip`). Everywhere else the pane stays live with a hole in it: a
 * clip on a Views pane, a window region on a Windows child window, a layer mask
 * on macOS.
 */
export function hidesPaneUnderOverlay(): boolean {
    if (getApi().getAboutModalDetails?.()?.paneClip && !clipUnavailable) return false;
    const ua = navigator.userAgent;
    return (ua.includes("Linux") && !ua.includes("Windows")) || isViewsPanesOnWindows();
}
