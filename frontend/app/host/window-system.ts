// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The window system the host's windows live in, read once at startup.
//
// The page can't tell native Wayland from X11 itself, and they differ in what
// a client may do:
// - no client can place its own window: setting a window's position does
//   nothing, and only the compositor can move it (`startDrag`);
// - a drag dropped outside every window is "rejected", and the compositor
//   animates its picture back to where the drag started.
//
// Drag handlers need the answer synchronously, so it is fetched at startup
// (`loadWindowSystem`, from initGlobal) and read with `isNativeWayland`. Until
// the answer arrives, and on hosts without the call, it is not native Wayland:
// the behaviour from before the host could say.

import { getApi } from "@/app/store/app-api";

type WindowSystem = "wayland" | "x11" | "windows" | "macos";

let windowSystem: WindowSystem | null = null;

/** Ask the host once. Safe to call more than once and on any host. */
export function loadWindowSystem(): Promise<void> {
    let ask: Promise<WindowSystem> | undefined;
    try {
        ask = getApi().getWindowSystem?.();
    } catch {
        return Promise.resolve();
    }
    if (!ask) return Promise.resolve();
    return ask.then(
        (ws) => {
            windowSystem = ws;
        },
        () => {}
    );
}

/** Is this host's window system native Wayland? False until known. */
export function isNativeWayland(): boolean {
    return windowSystem === "wayland";
}

/** Forget the answer, so tests start clean. */
export function resetWindowSystemForTest(): void {
    windowSystem = null;
}
