// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure helpers over a leaf's in-pane tab stack (`blockStack` /
 * `activeBlockId`). No imports beyond types, so both the stack mutators
 * (`layoutStack.ts`) and the backend-action handler (`layoutPersistence.ts`)
 * can share them without dragging each other's dependencies along.
 */

/** The node's stack, or `[blockId]` when it has none yet (back-compat: a
 *  non-stacked leaf behaves as a one-member stack). */
export function effectiveStack(data: TabLayoutData): string[] {
    return data.blockStack?.length ? data.blockStack : [data.blockId];
}

/**
 * Remove `blockId` from its leaf's stack in place, WITHOUT deleting its
 * block. If it was the visible tab, the right-hand neighbour becomes visible
 * (the new last member when it was rightmost — the editor tab strip's
 * convention). Returns false and changes nothing when `blockId` isn't a
 * member, or is the only one: removing the last tab means removing the leaf,
 * which is the caller's job. The caller commits the tree.
 */
export function removeMemberFromStack(data: TabLayoutData, blockId: string): boolean {
    const stack = effectiveStack(data);
    const idx = stack.indexOf(blockId);
    if (idx < 0 || stack.length <= 1) return false;

    const nextStack = stack.filter((id) => id !== blockId);
    data.blockStack = nextStack;
    if (data.activeBlockId === blockId || data.blockId === blockId) {
        const nextActive = nextStack[Math.min(idx, nextStack.length - 1)];
        data.activeBlockId = nextActive;
        data.blockId = nextActive;
    }
    return true;
}
