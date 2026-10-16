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
 * An entry is a path, or `{path, follow_folder}` for a media open (the
 * retired Media pane's way in: the tab follows newer files in its folder,
 * SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.11).
 *
 * Kept out of editor-model.ts so the batch rule is testable without building
 * an EditorViewModel. Returns the entries it opened, as they were in the
 * queue (pendingEntryKey tells them apart when the queue is read again).
 */

export interface PendingOpen {
    path: string;
    followFolder: boolean;
}

/** The file an entry opens, or null for an entry that isn't one. */
export function parsePendingEntry(entry: unknown): PendingOpen | null {
    if (typeof entry === "string") return entry.length > 0 ? { path: entry, followFolder: false } : null;
    if (entry && typeof entry === "object") {
        const e = entry as { path?: unknown; follow_folder?: unknown };
        if (typeof e.path === "string" && e.path.length > 0) return { path: e.path, followFolder: e.follow_folder === true };
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
    open: (path: string, opts: { followFolder: boolean }) => void,
    show: () => void
): unknown[] {
    const processed: unknown[] = [];
    for (const entry of pending) {
        const parsed = parsePendingEntry(entry);
        if (!parsed) continue;
        open(parsed.path, { followFolder: parsed.followFolder });
        processed.push(entry);
    }
    if (processed.length > 0) show();
    return processed;
}
