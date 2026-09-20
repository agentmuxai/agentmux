// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for stackMembers.ts's pure leaf-stack helpers.
 * `moveMemberInStack` mirrors the backend's `reorder_within_leaf`
 * (agentmux-srv/src/backend/layout/mod.rs) — see that function's own doc
 * comment for why a reorder must never change which member is visible
 * unless `activate` is explicitly requested.
 * Spec: docs/specs/SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1, Phase 3.
 */

import { describe, expect, it } from "vitest";
import {
    addMemberToStack,
    effectiveStack,
    moveMemberAcrossStacks,
    moveMemberInStack,
    removeMemberFromStack,
} from "./stackMembers";

function stacked(blockId: string, blockStack: string[], activeBlockId: string): TabLayoutData {
    return { blockId, blockStack, activeBlockId };
}

describe("moveMemberInStack", () => {
    it("reorders before the target", () => {
        const data = stacked("a", ["a", "b", "c"], "a");
        expect(moveMemberInStack(data, "c", "b", "before", false)).toBe(true);
        expect(data.blockStack).toEqual(["a", "c", "b"]);
    });

    it("reorders after the target", () => {
        const data = stacked("a", ["a", "b", "c"], "a");
        expect(moveMemberInStack(data, "a", "b", "after", false)).toBe(true);
        expect(data.blockStack).toEqual(["b", "a", "c"]);
    });

    it("reorders to the end, ignoring the target's own position", () => {
        const data = stacked("a", ["a", "b", "c"], "a");
        expect(moveMemberInStack(data, "a", "b", "end", false)).toBe(true);
        expect(data.blockStack).toEqual(["b", "c", "a"]);
    });

    it("does not change which tab is visible unless activate is set", () => {
        const data = stacked("b", ["a", "b", "c"], "b");
        expect(moveMemberInStack(data, "b", "c", "after", false)).toBe(true);
        expect(data.blockStack).toEqual(["a", "c", "b"]);
        expect(data.blockId).toBe("b");
        expect(data.activeBlockId).toBe("b");
    });

    it("activates the moved member when requested", () => {
        const data = stacked("a", ["a", "b", "c"], "a");
        expect(moveMemberInStack(data, "c", "a", "before", true)).toBe(true);
        expect(data.blockStack).toEqual(["c", "a", "b"]);
        expect(data.blockId).toBe("c");
        expect(data.activeBlockId).toBe("c");
    });

    it("returns false and changes nothing when blockId isn't a member", () => {
        const data = stacked("a", ["a", "b"], "a");
        const before = { ...data };
        expect(moveMemberInStack(data, "missing", "a", "after", false)).toBe(false);
        expect(data).toEqual(before);
    });

    it("returns false and changes nothing when targetBlockId isn't a member", () => {
        const data = stacked("a", ["a", "b"], "a");
        const before = { ...data };
        expect(moveMemberInStack(data, "a", "missing", "after", false)).toBe(false);
        expect(data).toEqual(before);
    });

    // ReAgent P2 on PR #3444: filtering blockId out of `members` when
    // blockId === targetBlockId also removes targetBlockId (same id), so
    // `next.indexOf(targetBlockId)` returns -1 and every position variant
    // below silently reorders the stack instead of leaving it unchanged.
    // Mirrors the backend's identical guard in move_stack_member
    // (agentmux-srv/src/backend/layout/mod.rs), added after the same
    // mistake there (ReAgent P1 on PR #3441) — never mirrored here until now.
    it("is a no-op — stack order unchanged — when blockId and targetBlockId are the same, for every position", () => {
        for (const position of ["before", "after", "end"] as const) {
            const data = stacked("a", ["a", "b", "c"], "a");
            expect(moveMemberInStack(data, "b", "b", position, false)).toBe(true);
            expect(data.blockStack).toEqual(["a", "b", "c"]);
        }
    });

    it("self-target still activates when requested, without reordering", () => {
        const data = stacked("a", ["a", "b", "c"], "a");
        expect(moveMemberInStack(data, "b", "b", "before", true)).toBe(true);
        expect(data.blockStack).toEqual(["a", "b", "c"]);
        expect(data.blockId).toBe("b");
        expect(data.activeBlockId).toBe("b");
    });
});

// SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.4 (cross-pane header-drop):
// moves blockId OUT of sourceData's stack and INTO targetData's stack.
// Mirrors the backend's cross-leaf branch of move_stack_member
// (remove_stack_member + push_stack_member_at composition) — but unlike
// that Rust function, this one only ever needs a plain append (the caller
// always targets "the destination pane's currently-active member", per
// §3.4's design; there is no drop-relative-to-a-specific-pill position for
// a whole-header drop), so there's no position/splice logic to get wrong
// here the way moveMemberInStack's self-target case did.
describe("moveMemberAcrossStacks", () => {
    it("moves the block from source to target, appended and activated", () => {
        const source = stacked("a", ["a", "b"], "a");
        const target = stacked("x", ["x", "y"], "x");
        expect(moveMemberAcrossStacks(source, target, "b", true)).toBe(true);
        expect(source.blockStack).toEqual(["a"]);
        expect(target.blockStack).toEqual(["x", "y", "b"]);
        expect(target.blockId).toBe("b");
        expect(target.activeBlockId).toBe("b");
    });

    it("does not activate the moved block in the target unless requested", () => {
        const source = stacked("a", ["a", "b"], "a");
        const target = stacked("x", ["x", "y"], "x");
        expect(moveMemberAcrossStacks(source, target, "b", false)).toBe(true);
        expect(target.blockStack).toEqual(["x", "y", "b"]);
        expect(target.blockId).toBe("x");
        expect(target.activeBlockId).toBe("x");
    });

    it("moving the currently-visible source member activates its right-hand neighbour there", () => {
        const source = stacked("b", ["a", "b", "c"], "b");
        const target = stacked("x", ["x"], "x");
        expect(moveMemberAcrossStacks(source, target, "b", false)).toBe(true);
        expect(source.blockStack).toEqual(["a", "c"]);
        expect(source.blockId).toBe("c");
        expect(source.activeBlockId).toBe("c");
    });

    it("promotes a single-block target into a real stack", () => {
        const source = stacked("a", ["a", "b"], "a");
        const target = stacked("x", [], "x");
        expect(moveMemberAcrossStacks(source, target, "b", true)).toBe(true);
        expect(target.blockStack).toEqual(["x", "b"]);
    });

    it("refuses — changes nothing — when blockId is the source's only member", () => {
        const source = stacked("a", [], "a");
        const target = stacked("x", ["x", "y"], "x");
        const sourceBefore = { ...source };
        const targetBefore = { ...target };
        expect(moveMemberAcrossStacks(source, target, "a", true)).toBe(false);
        expect(source).toEqual(sourceBefore);
        expect(target).toEqual(targetBefore);
    });

    it("refuses when blockId is not a member of the source at all", () => {
        const source = stacked("a", ["a", "b"], "a");
        const target = stacked("x", ["x"], "x");
        const targetBefore = { ...target };
        expect(moveMemberAcrossStacks(source, target, "not-there", true)).toBe(false);
        expect(target).toEqual(targetBefore);
    });
});

// Regression guard: confirms these two existing helpers are still exported
// and usable alongside the new one (moveMemberInStack composes with neither
// directly, but the cross-leaf case in layoutStack.ts, Phase 4, will).
describe("existing stack helpers still work", () => {
    it("effectiveStack / addMemberToStack / removeMemberFromStack", () => {
        const data = stacked("a", [], "a");
        expect(effectiveStack(data)).toEqual(["a"]);
        addMemberToStack(data, "b", false);
        expect(data.blockStack).toEqual(["a", "b"]);
        expect(removeMemberFromStack(data, "a")).toBe(true);
        expect(data.blockStack).toEqual(["b"]);
    });
});
