// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneConfirm } from "@/app/element/create-confirm";

export interface MemoryEditorKeyHandlers {
    /** Is an editor open right now? Keys are ignored otherwise. */
    isEditing: () => boolean;
    isDirty: () => boolean;
    onSave: () => void;
    onCancel: () => void;
    /** Asks before a dirty draft is thrown away; a view passes `askDiscard(its createConfirm())`. */
    confirm: (message: string) => boolean | Promise<boolean>;
}

/** The "Discard your unsaved changes?" question, drawn in the pane by the view's `createConfirm()`. */
export const askDiscard =
    (c: Pick<PaneConfirm, "ask">) =>
    (message: string): Promise<boolean> =>
        c.ask({ title: message, confirmLabel: "Discard", cancelLabel: "Keep editing" });

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
 *  cancelled (a promise of it when the answer is asked in the pane). */
export function requestCancel(h: Pick<MemoryEditorKeyHandlers, "isDirty" | "onCancel" | "confirm">): boolean | Promise<boolean> {
    const finish = (ok: boolean): boolean => {
        if (ok) h.onCancel();
        return ok;
    };
    if (!h.isDirty()) return finish(true);
    const answer = h.confirm("Discard your unsaved changes?");
    return typeof answer === "boolean" ? finish(answer) : answer.then(finish);
}
