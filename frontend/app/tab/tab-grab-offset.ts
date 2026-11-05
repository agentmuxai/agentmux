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
 * `tab-tearoff-rpc.ts::createTearOffTabAtRelease` and the cross-window
 * drag monitors. Deliberately NOT cleared on drop (see the comment in
 * `droppable-tab.tsx`'s `onDrop`); the next drag's preview overwrites it.
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

/**
 * From the cursor to the outer top-left of the window a torn-off tab opens, in DIP: the grab offset in the
 * tab, plus where the first tab sits in the window (its rect and the window's chrome). The window opens at
 * the cursor minus this, so the grabbed tab stays under the cursor. Null until a drag has set the grab offset
 * or while the strip has no tab. The release and the live follow (tab-live-follow.ts) both place by it.
 */
export function tabAnchorOffset(tabBarScroll: Element | null | undefined): TabGrabOffset | null {
    const grab = current;
    const firstTabRect = tabBarScroll?.querySelector(".tab-drop-wrapper")?.getBoundingClientRect();
    if (!grab || !firstTabRect) return null;
    const chromeBorderX = Math.max(0, window.outerWidth - window.innerWidth) / 2;
    const chromeBorderY = Math.max(0, window.outerHeight - window.innerHeight - chromeBorderX);
    return {
        x: grab.x + firstTabRect.left + chromeBorderX,
        y: grab.y + firstTabRect.top + chromeBorderY,
    };
}
