// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One batch of files an agent's `OpenEditor` call pushed into this existing
 * Editor pane (`editor:pending_open_files`, see `maybe_reuse_editor_pane` in
 * agentmux-srv's `app_api/pane.rs`). Opens each, then — once per batch, and
 * only if anything was opened — asks for the pane to be shown: an agent opens
 * a file so the user sees it, and a minimized, stacked-behind or
 * magnified-over pane shows nothing.
 * SPEC_EDITOR_REUSE_RESTORES_MINIMIZED_PANE_2026_09_30.md.
 *
 * An entry is a path, or `{path, follow_folder, viewer, line, offset, page}`:
 * `follow_folder` for a media open (the retired Media pane's way in: the tab
 * follows newer files in its folder, SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md
 * §4.11), `viewer` for an open that names the viewer (`muxsh edit`: text), and
 * where to land in the file: a text `line`, a hex `offset`, a PDF `page`.
 *
 * Kept out of editor-model.ts so the batch rule is testable without building
 * an EditorViewModel. Returns the entries it opened, as they were in the
 * queue (pendingEntryKey tells them apart when the queue is read again).
 */

import { whole } from "./open-position";
import { isViewerId, type ViewerId } from "./viewers/viewer-choice";

export interface PendingOpen {
    path: string;
    followFolder: boolean;
    /** Set when the open named a viewer; else the editor picks one. */
    viewer?: ViewerId;
    /** Where in the file to land, for the viewer that reads it. */
    line?: number;
    offset?: number;
    page?: number;
}

/** The file an entry opens, or null for an entry that isn't one. */
export function parsePendingEntry(entry: unknown): PendingOpen | null {
    if (typeof entry === "string") return entry.length > 0 ? { path: entry, followFolder: false } : null;
    if (entry && typeof entry === "object") {
        const e = entry as { path?: unknown; follow_folder?: unknown; viewer?: unknown; line?: unknown; offset?: unknown; page?: unknown };
        if (typeof e.path === "string" && e.path.length > 0) {
            const open: PendingOpen = { path: e.path, followFolder: e.follow_folder === true };
            if (isViewerId(e.viewer)) open.viewer = e.viewer;
            const line = whole(e.line, 1);
            const offset = whole(e.offset, 0);
            const page = whole(e.page, 1);
            if (line !== undefined) open.line = line;
            if (offset !== undefined) open.offset = offset;
            if (page !== undefined) open.page = page;
            return open;
        }
    }
    return null;
}

/** Compares entries across reads of the queue (objects aren't the same
 *  object twice). */
export function pendingEntryKey(entry: unknown): string {
    return typeof entry === "string" ? entry : JSON.stringify(entry);
}

export function openPendingFiles(
    pending: readonly unknown[],
    open: (path: string, opts: Omit<PendingOpen, "path">) => void,
    show: () => void
): unknown[] {
    const processed: unknown[] = [];
    for (const entry of pending) {
        const parsed = parsePendingEntry(entry);
        if (!parsed) continue;
        const { path, ...opts } = parsed;
        open(path, opts);
        processed.push(entry);
    }
    if (processed.length > 0) show();
    return processed;
}
