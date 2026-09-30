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
 * Kept out of editor-model.ts so the batch rule is testable without building
 * an EditorViewModel. Returns the paths it opened.
 */
export function openPendingFiles(
    pending: readonly unknown[],
    open: (path: string) => void,
    show: () => void
): string[] {
    const processed = pending.filter((p): p is string => typeof p === "string" && p.length > 0);
    for (const path of processed) open(path);
    if (processed.length > 0) show();
    return processed;
}
