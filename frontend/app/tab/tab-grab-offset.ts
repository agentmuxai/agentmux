// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Module-level signal for the cursor's offset within the tab element
 * at drag-start. Used by tear-off to position the new window so the
 * dragged tab lands under the cursor at the SAME offset the user
 * grabbed — Chrome-style "no teleport" handoff.
 *
 * Set in `droppable-tab.tsx::onGenerateDragPreview` (the earliest
 * pragmatic-dnd hook that exposes `DragLocation`); read in
 * `tabbar.tsx::performTabTearOff` and the cross-window drag
 * monitors. Cleared on drop / drag end.
 *
 * Spec: docs/specs/SPEC_TAB_TEAROFF_POSITION_AND_PAINT_2026-05-07.md
 */

export interface TabGrabOffset {
    /** Pixels from the LEFT edge of the tab element to the cursor. */
    x: number;
    /** Pixels from the TOP edge of the tab element to the cursor. */
    y: number;
}

let current: TabGrabOffset | null = null;

export function setTabGrabOffset(offset: TabGrabOffset | null): void {
    current = offset;
}

export function getTabGrabOffset(): TabGrabOffset | null {
    return current;
}
