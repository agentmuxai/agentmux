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

/**
 * Add `blockId` to its leaf's stack in place (a leaf with no stack becomes a
 * two-member stack), optionally as the visible tab. Mirrors the backend's
 * `push_stack_member` (`agentmux-srv/src/backend/layout/mod.rs`), so a
 * `stackpush` action from `CreateBlockInStack` lands the same way on both
 * sides. The caller commits the tree.
 * SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.3.
 */
export function addMemberToStack(data: TabLayoutData, blockId: string, activate: boolean): void {
    const stack = effectiveStack(data);
    data.blockStack = stack.includes(blockId) ? [...stack] : [...stack, blockId];
    if (!data.activeBlockId) data.activeBlockId = data.blockId;
    if (activate) {
        data.activeBlockId = blockId;
        data.blockId = blockId;
    }
}

/**
 * Splice `blockId` to `position` relative to `targetBlockId` within the SAME
 * leaf's stack. Both must already be members. Mirrors the backend's
 * `reorder_within_leaf` (`agentmux-srv/src/backend/layout/mod.rs`) exactly,
 * including the one subtlety that makes this a distinct function rather than
 * a `removeMemberFromStack` + `addMemberToStack` composition: only touches
 * `blockId`/`activeBlockId` when `activate` is explicitly set — reordering
 * the currently-visible member must not switch away from it, which
 * `removeMemberFromStack`'s neighbour-reassignment would otherwise do.
 * Returns `false` and changes nothing when either id isn't a member.
 * SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1, Phase 3.
 */
export function moveMemberInStack(
    data: TabLayoutData,
    blockId: string,
    targetBlockId: string,
    position: "before" | "after" | "end",
    activate: boolean
): boolean {
    const members = effectiveStack(data);
    if (!members.includes(blockId) || !members.includes(targetBlockId)) return false;

    const next = members.filter((id) => id !== blockId);
    const targetIdx = next.indexOf(targetBlockId);
    const insertAt = position === "before" ? targetIdx : position === "after" ? targetIdx + 1 : next.length;
    next.splice(insertAt, 0, blockId);

    data.blockStack = next;
    if (activate) {
        data.activeBlockId = blockId;
        data.blockId = blockId;
    }
    return true;
}
