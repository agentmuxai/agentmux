// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The toolbar's Reload button is a Stop button while the page is loading, as in
 * Chrome: one press stops the load, and it is Reload again when the load ends.
 * The glyph, tooltip and accessible name change together, so a screen reader
 * hears what a press will do.
 */

export interface ReloadButtonState {
    /** What a press does. */
    action: "reload" | "stop";
    glyph: string;
    /** Both the tooltip and the accessible name. */
    label: string;
}

export function reloadButtonState(loading: boolean): ReloadButtonState {
    return loading
        ? { action: "stop", glyph: "✕", label: "Stop loading" }
        : { action: "reload", glyph: "↻", label: "Reload" };
}
