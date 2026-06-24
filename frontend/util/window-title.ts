// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Window-title resolution shared between the OS window title (driven from
 * `app-init.ts`) and the bottom-right InstancePanel that lists open windows.
 *
 * Spec: docs/specs/SPEC_WINDOW_TITLE_FORMAT_2026-05-13.md
 */

const TITLE_SEPARATOR = " - ";
const APP_NAME = "AgentMux";

/** Key under `WaveWindow.meta` where the user-set instance name is stored. 64-char max. */
export const DISPLAY_NAME_META_KEY = "window:displayname";
export const DISPLAY_NAME_MAX_LEN = 64;

export interface ResolveWindowNameOpts {
    /** Value of `WaveWindow.meta["window:displayname"]` if set. */
    displayName?: string | null;
    /** Value of the assigned workspace's `name` if any. */
    workspaceName?: string | null;
    /** 0-indexed position in `openWindowEntriesAtom`; rendered as "Window N" where N = index + 1. */
    indexInOpenWindows: number;
}

/**
 * Three-tier resolution matching the InstancePanel rules:
 *   1. user-set display name (trimmed; empty falls through)
 *   2. workspace name (trimmed; empty falls through)
 *   3. positional fallback "Window N"
 */
export function resolveWindowName(opts: ResolveWindowNameOpts): string {
    const display = (opts.displayName ?? "").trim();
    if (display) return display;
    const ws = (opts.workspaceName ?? "").trim();
    if (ws) return ws;
    return `Window ${opts.indexInOpenWindows + 1}`;
}

export interface ResolveFloatingPaneNameOpts {
    /** Value of `workspace.name` for the workspace hosted in this pane. */
    workspaceName?: string | null;
    /** Human-readable label derived from the pane's block (e.g. view type or agent name). */
    blockViewLabel?: string | null;
    /** 0-indexed position in `openFloatingPaneEntriesAtom`; rendered as "Pane N" where N = index + 1. */
    indexInOpenPanes: number;
}

/**
 * Two-tier resolution for floating pane display names:
 *   1. block view label (e.g. "Agent", "Terminal", "Browser")
 *   2. workspace name
 *   3. positional fallback "Pane N"
 */
export function resolveFloatingPaneName(opts: ResolveFloatingPaneNameOpts): string {
    const view = (opts.blockViewLabel ?? "").trim();
    if (view) return view;
    const ws = (opts.workspaceName ?? "").trim();
    if (ws) return ws;
    return `Pane ${opts.indexInOpenPanes + 1}`;
}

/**
 * Format the OS window title. Tab name omitted (no empty middle slot) when
 * absent, since not every init path has a tab loaded yet.
 *
 *   formatWindowTitle("Main", "Shell")  → "Main - Shell - AgentMux"
 *   formatWindowTitle("Main", "")       → "Main - AgentMux"
 *   formatWindowTitle("Main", undefined)→ "Main - AgentMux"
 */
export function formatWindowTitle(windowName: string, tabName: string | null | undefined): string {
    const tab = (tabName ?? "").trim();
    if (tab) {
        return `${windowName}${TITLE_SEPARATOR}${tab}${TITLE_SEPARATOR}${APP_NAME}`;
    }
    return `${windowName}${TITLE_SEPARATOR}${APP_NAME}`;
}
