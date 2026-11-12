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

/**
 * Whether the host hides a browser pane whole while a DOM overlay (a menu, a
 * tooltip, a modal) is over it, so the page shows a frozen frame of it instead
 * (use-freeze-frame.ts). Only a Views pane on Linux or Windows whose host does
 * not cut the pane around the overlay (`paneClip` in the about details:
 * AGENTMUX_PANE_CLIP=off). Everywhere else the pane stays live with a hole in
 * it: a clip on a Views pane, a window region on a Windows child window, a
 * layer mask on macOS.
 */
export function hidesPaneUnderOverlay(): boolean {
    if (getApi().getAboutModalDetails?.()?.paneClip) return false;
    const ua = navigator.userAgent;
    return (ua.includes("Linux") && !ua.includes("Windows")) || isViewsPanesOnWindows();
}
