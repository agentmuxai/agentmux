// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An arrow key held with Shift, Ctrl, Alt or Cmd. In a text box these select, or move by word or line:
 * the browser owns them. History recall, the completion lists and the ghost-text suggestion act only on a
 * bare arrow, so none of them can take a selection key (Shift+Down after a recall moved to the next message).
 */
export function isModifiedArrow(e: KeyboardEvent): boolean {
    return e.key.startsWith("Arrow") && (e.shiftKey || e.ctrlKey || e.altKey || e.metaKey);
}
