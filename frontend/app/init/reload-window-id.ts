// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The srv window id this renderer had before a reload.
 *
 * A window other than the main one creates a new srv window row each time
 * its page loads, including on a reload, so the row's own theme and opacity
 * would stay behind on the old row. The renderer remembers its row id in
 * sessionStorage, which survives a reload, and hands it to `CreateWindow`,
 * which copies the appearance onto the new row
 * (SPEC_PER_WINDOW_THEME_AND_OPACITY_2026_10_12 §4.6).
 *
 * A pool renderer keeps its sessionStorage when it is demoted and promoted
 * again for a different window, so the pool's boot path forgets the id.
 */

const KEY = "agentmux:previous-window-id";

/** The id this renderer's window had before the page was reloaded, if any. */
export function previousWindowId(): string | undefined {
    try {
        return sessionStorage.getItem(KEY) || undefined;
    } catch {
        return undefined;
    }
}

/** Remember this renderer's window id, for the next reload. */
export function rememberWindowId(windowId: string): void {
    try {
        sessionStorage.setItem(KEY, windowId);
    } catch {
        // No sessionStorage: a reload falls back to the defaults.
    }
}

/** Forget it: this renderer is about to become a different window. */
export function forgetWindowId(): void {
    try {
        sessionStorage.removeItem(KEY);
    } catch {
        // Nothing to forget.
    }
}
