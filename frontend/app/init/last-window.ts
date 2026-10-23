// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a window that isn't the main one should bring back the last window
 * closed (SPEC_WORKSPACES_2026_10_23.md §2) instead of opening the default
 * panes.
 *
 * With "Keep running in the tray" (`app:runinbackground`) on, closing the
 * last window doesn't quit: srv keeps running with no window, and that
 * window stays open in the workspace, its contents kept. The window the tray
 * (or a second launch, or the macOS dock) opens next is a new window, not
 * "main", so without this it got the default panes, and the next save
 * replaced the workspace's window with them. It asks srv to restore the
 * workspace instead, as the main window does at a launch; srv checks again
 * that no window is open.
 *
 * Only a plain new window: one opened onto given contents (a tear-off, a
 * closed window reopening, a toast's pane), or with a pane of its own (a
 * widget's "Open in New Window", an approval page), opens what it was asked
 * for; and a reload keeps its own window.
 */
export function restoresLastWindow(opts: {
    /** `Client.windowids` as this window found it. */
    windowIds: string[] | null | undefined;
    /** The contents it was opened onto (`workspaceId` in its URL), if any. */
    workspaceId: string;
    /** The pane it was opened with, from the pool's promote or its URL. */
    view: string | null | undefined;
    /** A reload found its own window again. */
    reattached: boolean;
}): boolean {
    return !opts.reattached && !opts.workspaceId && !opts.view && (opts.windowIds?.length ?? 0) === 0;
}
