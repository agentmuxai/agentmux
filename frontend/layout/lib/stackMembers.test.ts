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
import { addMemberToStack, effectiveStack, moveMemberInStack, removeMemberFromStack } from "./stackMembers";

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
