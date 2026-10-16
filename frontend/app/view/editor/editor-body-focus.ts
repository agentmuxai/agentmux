// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where the keyboard goes when it leaves the file tree for a body that isn't
 * the text editor: a viewer's own focusable control if it has one (the image
 * viewer's canvas), else the viewer or the markdown preview itself, made
 * focusable from code only (`tabindex=-1`, so Tab doesn't stop there).
 * Without that, an info card, audio/video or a markdown preview has nothing
 * that takes focus and the keyboard stays in the tree.
 * docs/specs/SPEC_EDITOR_TREE_KEYBOARD_ICONS_AND_CLICK_ZOOM_2026_10_16.md §5.4.
 */
export function bodyFocusTarget(root: ParentNode): HTMLElement | null {
    const own = root.querySelector<HTMLElement>('.editor-body-wrap [tabindex="0"]');
    if (own) return own;
    const body = root.querySelector<HTMLElement>(".editor-body-wrap .editor-viewer, .editor-preview-content");
    if (!body) return null;
    if (!body.hasAttribute("tabindex")) body.tabIndex = -1;
    return body;
}
