// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/util/app-api";

/**
 * Whether browser panes in this window are CEF Views overlays on Windows
 * (AGENTMUX_PANE_BACKEND=views; the host reports it as `paneBackend` in the
 * about details). Such a pane is hidden whole while a DOM overlay is over it
 * (a frozen frame of the page is shown instead: use-freeze-frame.ts), where a
 * Windows child-window pane gets a hole cut out of it. Linux has the same
 * hide-whole behaviour without this setting, and macOS punches a hole.
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
