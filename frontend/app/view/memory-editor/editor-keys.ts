// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export interface MemoryEditorKeyHandlers {
    /** Is an editor open right now? Keys are ignored otherwise. */
    isEditing: () => boolean;
    isDirty: () => boolean;
    onSave: () => void;
    onCancel: () => void;
    /** Injectable for tests; defaults to `window.confirm`. */
    confirm?: (message: string) => boolean;
}

/**
 * Keyboard contract for every memory editor surface
 * (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4): Ctrl/Cmd+S saves, Esc
 * cancels — asking first when the draft is dirty. Attached as `onKeyDown` on
 * the editor surface's root so it only fires with focus inside it, and it
 * stops propagation for the keys it handles so a hosting modal's own Esc
 * doesn't also close the pane underneath an unsaved draft.
 */
export function handleMemoryEditorKeyDown(e: KeyboardEvent, h: MemoryEditorKeyHandlers): void {
    if (!h.isEditing()) return;
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "s") {
        e.preventDefault();
        e.stopPropagation();
        h.onSave();
        return;
    }
    if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        requestCancel(h);
    }
}

/** Cancel an edit, asking first when the draft is dirty — Esc and the
 *  Cancel button share this so they can't disagree. Returns whether it
 *  cancelled. */
export function requestCancel(h: Pick<MemoryEditorKeyHandlers, "isDirty" | "onCancel" | "confirm">): boolean {
    const confirmFn = h.confirm ?? ((m: string) => window.confirm(m));
    if (h.isDirty() && !confirmFn("Discard your unsaved changes?")) return false;
    h.onCancel();
    return true;
}
