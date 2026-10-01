// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The value of `window:transparent` the host started with, from the
 * `window_transparent=0|1` query param it puts on the main window's URL
 * (crates/cef/src/app/mod.rs). `null` when the param is absent (other windows).
 */
export function startupWindowTransparent(search: string = window.location.search): boolean | null {
    const v = new URLSearchParams(search).get("window_transparent");
    if (v === "1") return true;
    if (v === "0") return false;
    return null;
}

/**
 * On Linux the host picks XWayland vs native Wayland, the compositing flags and
 * the page background alpha once, before CefInitialize, from `window:transparent`.
 * Turning transparency on in a session that started without it can't take effect
 * until AgentMux restarts (#4011).
 */
export function transparencyNeedsRestart(o: { linux: boolean; startup: boolean | null; transparent: boolean }): boolean {
    return o.linux && o.startup === false && o.transparent;
}
