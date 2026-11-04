// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/util/app-api";

/**
 * Whether browser panes here are CEF Views overlays on Windows
 * (AGENTMUX_PANE_BACKEND=views; the host reports it as `paneBackend` in the
 * about details). Such a pane is hidden whole while a DOM overlay is over it
 * (a frozen frame of the page is shown instead: use-freeze-frame.ts), where a
 * Windows child-window pane gets a hole cut out of it. Linux has the same
 * hide-whole behaviour without this setting, and macOS punches a hole.
 */
export function isViewsPanesOnWindows(): boolean {
    return navigator.userAgent.includes("Windows") && getApi().getAboutModalDetails?.()?.paneBackend === "views";
}
