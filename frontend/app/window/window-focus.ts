// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Window-focus signal — reactive accessor for "is the OS window in
 * foreground AND visible?". Used by the sound-notifications subsystem
 * (SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §4.7) to suppress sound when
 * the user is already looking at the originating pane in the focused
 * window. Self-contained — the only consumer in v1 is the sound
 * service, but the signal is intentionally general so future
 * consumers can subscribe without re-implementing it.
 *
 * Combines `document.hasFocus()` (DOM focus) and
 * `document.visibilityState === "visible"` (tab/window visibility) —
 * both must hold for the signal to be true. A minimized window will
 * report `document.hasFocus() === false`, and a backgrounded tab
 * reports `visibilityState === "hidden"`.
 */

import { createSignal, type Accessor } from "solid-js";

let cached: Accessor<boolean> | null = null;

function compute(): boolean {
    if (typeof document === "undefined" || typeof window === "undefined") return true;
    return document.hasFocus() && document.visibilityState === "visible";
}

/**
 * Returns a SolidJS accessor that reactively tracks whether the
 * current window is OS-focused and visible. The first call wires
 * up `focus` / `blur` / `visibilitychange` listeners; subsequent
 * calls return the cached accessor (idempotent).
 */
export function makeWindowFocusSignal(): Accessor<boolean> {
    if (cached) return cached;
    const [focused, setFocused] = createSignal(compute());
    const update = () => setFocused(compute());
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    document.addEventListener("visibilitychange", update);
    cached = focused;
    return focused;
}
