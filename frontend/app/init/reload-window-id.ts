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
 *
 * When that row is still this window's, a reload re-attaches to it instead
 * (`reattachableWindow`): a new row each load also left the old one behind,
 * and a window with no `workspaceId` in its URL (one promoted from the pool
 * for "New Window") came back showing a new, empty workspace
 * (SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §2).
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

/**
 * The row this renderer had before a reload, if the window can re-attach to
 * it: it still exists, is still open (in the client's `windowids`), is still
 * this window's (its `host:label` crumb is `label`) and its workspace still
 * exists. Otherwise undefined, and the caller creates a row as before. A
 * failed lookup counts as "can't".
 */
export async function reattachableWindow(
    openWindowIds: string[] | undefined,
    label: string,
    getWindow: (id: string) => Promise<MuxWindow | null | undefined>,
    getWorkspace: (id: string) => Promise<unknown>
): Promise<MuxWindow | undefined> {
    const prev = previousWindowId();
    if (!prev || !label || !openWindowIds?.includes(prev)) return undefined;
    try {
        const win = await getWindow(prev);
        if (!win || win.meta?.["host:label"] !== label || !win.workspaceid) return undefined;
        return (await getWorkspace(win.workspaceid)) ? win : undefined;
    } catch {
        return undefined;
    }
}
