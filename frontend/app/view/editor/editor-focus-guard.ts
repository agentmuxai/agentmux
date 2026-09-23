// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

const EDITABLE_CONTROL = 'input, textarea, select, [contenteditable=""], [contenteditable="true"]';

/**
 * True when the user already has DOM focus on an editable control inside
 * `root` — the file tree's rename/filter input, for example — as opposed to
 * CodeMirror itself or nothing at all.
 *
 * Why this exists (ReAgent P1 on PR #3519): editor-view.tsx rebuilds the
 * CodeMirror instance not only when the user switches into the pane or
 * changes its in-pane tab, but also whenever `model.contentAtom()` changes —
 * deliberately, so an external edit to a clean file reloads the buffer
 * (SPEC_EDITOR_LIVE_FILE_RELOAD). The auto-focus claim that runs after each
 * rebuild only checks block-level focus ("is this the active tab's focused
 * pane"), which is still true while the user types in the file tree, so an
 * external reload would have yanked the caret into CodeMirror mid-keystroke.
 *
 * The destroyed CodeMirror is never matched here: its contenteditable is
 * removed from the DOM before the rebuild, so focus that lived there has
 * already fallen back to `document.body`, which is outside `root`. That is
 * what keeps "restore the caret after a reload" working.
 */
export function userIsTypingElsewhereIn(root: Element | null | undefined, active: Element | null | undefined): boolean {
    if (!root || !active || active === root) return false;
    if (!root.contains(active)) return false;
    return active.matches(EDITABLE_CONTROL);
}
