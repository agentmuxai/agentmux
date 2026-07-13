// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    groupCacheKey,
    groupSubagentsByWorkflow,
    isNameGroup,
    isWorkflowGroup,
    mergeSubagentsPreservingIdentity,
    pruneGroupIdentityCache,
    stabilizeGroupIdentity,
    type ActiveSubagent,
    type NameGroup,
    type SwarmChild,
    type WorkflowGroup,
} from "./swarm-model";

/** Extract a SwarmChild's own identifying id, for assertions that don't care
 *  which of the three variants they got — narrows through all three via
 *  isWorkflowGroup/isNameGroup rather than an unsafe cast. */
function childId(c: SwarmChild): string {
    if (isWorkflowGroup(c)) return c.workflowId;
    if (isNameGroup(c)) return c.name;
    return c.agent_id;
}

function mk(overrides: Partial<ActiveSubagent> & Pick<ActiveSubagent, "agent_id">): ActiveSubagent {
    return {
        slug: "",
        parent_agent: "parent",
        parent_block_id: "block-1",
        session_id: "session-1",
        status: "completed",
        spawned_at: 0,
        last_event_at: 0,
        event_count: 1,
        model: null,
        workflow_id: null,
        display_name: null,
        ...overrides,
    };
}

describe("groupSubagentsByWorkflow", () => {
    it("leaves subagents with no workflow_id as loose, unrouped rows", () => {
        const result = groupSubagentsByWorkflow([
            mk({ agent_id: "a1", workflow_id: null }),
            mk({ agent_id: "a2", workflow_id: null }),
        ]);
        expect(result).toHaveLength(2);
        expect(result.every((c) => !isWorkflowGroup(c))).toBe(true);
    });

    it("collapses subagents sharing a workflow_id into one group", () => {
        const result = groupSubagentsByWorkflow([
            mk({ agent_id: "a1", workflow_id: "wf_1", slug: "cheerful-enchanting-sketch" }),
            mk({ agent_id: "a2", workflow_id: "wf_1", slug: "cheerful-enchanting-sketch" }),
            mk({ agent_id: "a3", workflow_id: "wf_1", slug: "cheerful-enchanting-sketch" }),
        ]);
        expect(result).toHaveLength(1);
        const group = result[0];
        expect(isWorkflowGroup(group)).toBe(true);
        if (isWorkflowGroup(group)) {
            expect(group.totalCount).toBe(3);
            expect(group.name).toBe("cheerful-enchanting-sketch");
        }
    });

    it("mixes loose subagents and groups independently, one group per distinct workflow_id", () => {
        const result = groupSubagentsByWorkflow([
            mk({ agent_id: "a1", workflow_id: "wf_1" }),
            mk({ agent_id: "a2", workflow_id: "wf_1" }),
            mk({ agent_id: "a3", workflow_id: "wf_2" }),
            mk({ agent_id: "a4", workflow_id: null }),
        ]);
        const groups = result.filter(isWorkflowGroup);
        const loose = result.filter((c) => !isWorkflowGroup(c));
        expect(groups).toHaveLength(2);
        expect(loose).toHaveLength(1);
    });

    it("marks a group active if any member is still active", () => {
        const result = groupSubagentsByWorkflow([
            mk({ agent_id: "a1", workflow_id: "wf_1", status: "completed" }),
            mk({ agent_id: "a2", workflow_id: "wf_1", status: "active" }),
        ]);
        const group = result[0];
        if (isWorkflowGroup(group)) {
            expect(group.status).toBe("active");
            expect(group.activeCount).toBe(1);
        } else {
            throw new Error("expected a workflow group");
        }
    });

    it("marks a group retired only once every member has completed", () => {
        const result = groupSubagentsByWorkflow([
            mk({ agent_id: "a1", workflow_id: "wf_1", status: "completed" }),
            mk({ agent_id: "a2", workflow_id: "wf_1", status: "completed" }),
        ]);
        const group = result[0];
        if (isWorkflowGroup(group)) {
            expect(group.status).toBe("retired");
            expect(group.activeCount).toBe(0);
        } else {
            throw new Error("expected a workflow group");
        }
    });

    it("treats an abandoned member the same as a completed one for grouping purposes (both terminal)", () => {
        // SubagentStatus::Abandoned (SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md)
        // — a subagent whose parent turn ended without a Result line. Like
        // "completed", it must NOT count toward activeCount and must let the
        // group retire.
        const result = groupSubagentsByWorkflow([
            mk({ agent_id: "a1", workflow_id: "wf_1", status: "completed" }),
            mk({ agent_id: "a2", workflow_id: "wf_1", status: "abandoned" }),
        ]);
        const group = result[0];
        if (isWorkflowGroup(group)) {
            expect(group.status).toBe("retired");
            expect(group.activeCount).toBe(0);
            expect(group.totalCount).toBe(2);
        } else {
            throw new Error("expected a workflow group");
        }
    });

    it("derives the group name from the first member with a non-empty slug", () => {
        const result = groupSubagentsByWorkflow([
            mk({ agent_id: "a1", workflow_id: "wf_1", slug: "", last_event_at: 200 }),
            mk({ agent_id: "a2", workflow_id: "wf_1", slug: "zesty-crafting-kahan", last_event_at: 100 }),
        ]);
        const group = result[0];
        if (isWorkflowGroup(group)) {
            expect(group.name).toBe("zesty-crafting-kahan");
        } else {
            throw new Error("expected a workflow group");
        }
    });

    it("falls back to the workflow id as the name when no member has a slug", () => {
        const result = groupSubagentsByWorkflow([
            mk({ agent_id: "a1", workflow_id: "wf_unnamed", slug: "" }),
        ]);
        const group = result[0];
        if (isWorkflowGroup(group)) {
            expect(group.name).toBe("wf_unnamed");
        } else {
            throw new Error("expected a workflow group");
        }
    });

    it("sorts loose subagents and groups together by most recent activity", () => {
        const result = groupSubagentsByWorkflow([
            mk({ agent_id: "old-loose", workflow_id: null, last_event_at: 100 }),
            mk({ agent_id: "a1", workflow_id: "wf_1", last_event_at: 500 }),
            mk({ agent_id: "newest-loose", workflow_id: null, last_event_at: 900 }),
        ]);
        expect(result).toHaveLength(3);
        // newest-loose (900) > wf_1 group (500) > old-loose (100)
        expect(childId(result[0])).toBe("newest-loose");
        expect(childId(result[2])).toBe("old-loose");
    });

    describe("name-based grouping of loose subagents (issue: same-name duplicate rows)", () => {
        it("collapses 2+ loose subagents sharing a display_name into one NameGroup", () => {
            const result = groupSubagentsByWorkflow([
                mk({ agent_id: "a1", workflow_id: null, display_name: "Code Reviewer" }),
                mk({ agent_id: "a2", workflow_id: null, display_name: "Code Reviewer" }),
                mk({ agent_id: "a3", workflow_id: null, display_name: "Code Reviewer" }),
            ]);
            expect(result).toHaveLength(1);
            const group = result[0];
            expect(isNameGroup(group)).toBe(true);
            if (isNameGroup(group)) {
                expect(group.name).toBe("Code Reviewer");
                expect(group.totalCount).toBe(3);
            }
        });

        it("leaves a single subagent with a unique display_name as a loose row (no group chrome for a non-dupe)", () => {
            const result = groupSubagentsByWorkflow([
                mk({ agent_id: "a1", workflow_id: null, display_name: "Only One" }),
            ]);
            expect(result).toHaveLength(1);
            expect(isNameGroup(result[0])).toBe(false);
            expect(childId(result[0])).toBe("a1");
        });

        it("leaves subagents with no display_name yet as loose, ungrouped rows even if several exist", () => {
            const result = groupSubagentsByWorkflow([
                mk({ agent_id: "a1", workflow_id: null, display_name: null }),
                mk({ agent_id: "a2", workflow_id: null, display_name: null }),
            ]);
            expect(result).toHaveLength(2);
            expect(result.every((c) => !isNameGroup(c))).toBe(true);
        });

        it("workflow grouping takes priority — same-named subagents in a workflow never form a NameGroup", () => {
            const result = groupSubagentsByWorkflow([
                mk({ agent_id: "a1", workflow_id: "wf_1", display_name: "Worker" }),
                mk({ agent_id: "a2", workflow_id: "wf_1", display_name: "Worker" }),
            ]);
            expect(result).toHaveLength(1);
            expect(isWorkflowGroup(result[0])).toBe(true);
            expect(result.some(isNameGroup)).toBe(false);
        });

        it("marks a NameGroup active if any member is still active", () => {
            const result = groupSubagentsByWorkflow([
                mk({ agent_id: "a1", workflow_id: null, display_name: "N", status: "completed" }),
                mk({ agent_id: "a2", workflow_id: null, display_name: "N", status: "active" }),
            ]);
            const group = result[0];
            if (isNameGroup(group)) {
                expect(group.status).toBe("active");
                expect(group.activeCount).toBe(1);
            } else {
                throw new Error("expected a name group");
            }
        });

        it("marks a NameGroup retired only once every member has completed", () => {
            const result = groupSubagentsByWorkflow([
                mk({ agent_id: "a1", workflow_id: null, display_name: "N", status: "completed" }),
                mk({ agent_id: "a2", workflow_id: null, display_name: "N", status: "completed" }),
            ]);
            const group = result[0];
            if (isNameGroup(group)) {
                expect(group.status).toBe("retired");
            } else {
                throw new Error("expected a name group");
            }
        });

        it("treats an abandoned NameGroup member the same as a completed one (both terminal)", () => {
            const result = groupSubagentsByWorkflow([
                mk({ agent_id: "a1", workflow_id: null, display_name: "N", status: "completed" }),
                mk({ agent_id: "a2", workflow_id: null, display_name: "N", status: "abandoned" }),
            ]);
            const group = result[0];
            if (isNameGroup(group)) {
                expect(group.status).toBe("retired");
                expect(group.activeCount).toBe(0);
            } else {
                throw new Error("expected a name group");
            }
        });

        it("mixes loose subagents, workflow groups, and name groups together, sorted by recency", () => {
            const result = groupSubagentsByWorkflow([
                mk({ agent_id: "solo", workflow_id: null, display_name: "Unique", last_event_at: 50 }),
                mk({ agent_id: "n1", workflow_id: null, display_name: "Dup", last_event_at: 300 }),
                mk({ agent_id: "n2", workflow_id: null, display_name: "Dup", last_event_at: 700 }),
                mk({ agent_id: "w1", workflow_id: "wf_1", last_event_at: 500 }),
            ]);
            expect(result).toHaveLength(3);
            const nameGroups = result.filter(isNameGroup);
            const workflowGroups = result.filter(isWorkflowGroup);
            expect(nameGroups).toHaveLength(1);
            expect(workflowGroups).toHaveLength(1);
            // Dup group's lastEventAt (700, from n2) > wf_1 (500) > solo (50)
            expect(childId(result[0])).toBe("Dup");
            expect(childId(result[2])).toBe("solo");
        });
    });
});

describe("mergeSubagentsPreservingIdentity", () => {
    it("reuses the old object reference for an entry whose fields are unchanged", () => {
        const prev = [mk({ agent_id: "a1", slug: "foo" })];
        const next = [mk({ agent_id: "a1", slug: "foo" })];
        const merged = mergeSubagentsPreservingIdentity(prev, next);
        expect(merged[0]).toBe(prev[0]);
    });

    it("uses the new object when a field actually changed", () => {
        const prev = [mk({ agent_id: "a1", status: "active" })];
        const next = [mk({ agent_id: "a1", status: "completed" })];
        const merged = mergeSubagentsPreservingIdentity(prev, next);
        expect(merged[0]).toBe(next[0]);
        expect(merged[0].status).toBe("completed");
    });

    it("only replaces the changed entry, leaving unrelated entries' references untouched", () => {
        const prev = [
            mk({ agent_id: "a1", status: "active" }),
            mk({ agent_id: "a2", status: "active" }),
        ];
        const next = [
            mk({ agent_id: "a1", status: "completed" }), // changed
            mk({ agent_id: "a2", status: "active" }), // unchanged
        ];
        const merged = mergeSubagentsPreservingIdentity(prev, next);
        expect(merged[0]).toBe(next[0]);
        expect(merged[1]).toBe(prev[1]);
    });

    it("uses the new object for a subagent with no prior entry", () => {
        const prev: ActiveSubagent[] = [];
        const next = [mk({ agent_id: "a1" })];
        const merged = mergeSubagentsPreservingIdentity(prev, next);
        expect(merged[0]).toBe(next[0]);
    });
});

describe("stabilizeGroupIdentity", () => {
    it("reuses the cached WorkflowGroup reference when nothing about the group changed", () => {
        const cache = new Map<string, WorkflowGroup>();
        const members = [mk({ agent_id: "a1", workflow_id: "wf_1" })];
        const first = groupSubagentsByWorkflow(members);
        const stabilizedFirst = stabilizeGroupIdentity(cache, first);

        // A second, independently-built (but content-identical) group for the
        // same workflow — mirrors what a fresh groupSubagentsByWorkflow() call
        // produces on an unrelated buildTree() recompute.
        const second = groupSubagentsByWorkflow(members);
        const stabilizedSecond = stabilizeGroupIdentity(cache, second);

        expect(stabilizedSecond[0]).toBe(stabilizedFirst[0]);
    });

    it("returns a fresh reference when the group's member set actually changed", () => {
        const cache = new Map<string, WorkflowGroup>();
        const first = groupSubagentsByWorkflow([mk({ agent_id: "a1", workflow_id: "wf_1", status: "active" })]);
        const stabilizedFirst = stabilizeGroupIdentity(cache, first);

        const second = groupSubagentsByWorkflow([mk({ agent_id: "a1", workflow_id: "wf_1", status: "completed" })]);
        const stabilizedSecond = stabilizeGroupIdentity(cache, second);

        expect(stabilizedSecond[0]).not.toBe(stabilizedFirst[0]);
    });

    it("passes loose (non-group) subagents through unchanged", () => {
        const cache = new Map<string, WorkflowGroup>();
        const loose = groupSubagentsByWorkflow([mk({ agent_id: "a1", workflow_id: null })]);
        const stabilized = stabilizeGroupIdentity(cache, loose);
        expect(stabilized[0]).toBe(loose[0]);
        expect(cache.size).toBe(0);
    });

    it("reuses the cached NameGroup reference when nothing about the group changed", () => {
        const cache = new Map<string, WorkflowGroup | NameGroup>();
        const members = [
            mk({ agent_id: "a1", workflow_id: null, display_name: "Dup" }),
            mk({ agent_id: "a2", workflow_id: null, display_name: "Dup" }),
        ];
        const first = groupSubagentsByWorkflow(members);
        const stabilizedFirst = stabilizeGroupIdentity(cache, first);

        const second = groupSubagentsByWorkflow(members);
        const stabilizedSecond = stabilizeGroupIdentity(cache, second);

        expect(stabilizedSecond[0]).toBe(stabilizedFirst[0]);
    });

    it("keeps a WorkflowGroup and a NameGroup with the same raw id/name as distinct cache entries", () => {
        // groupCacheKey namespaces with "wf:"/"name:" precisely so this
        // coincidence can't collide two unrelated groups into one cache slot.
        const cache = new Map<string, WorkflowGroup | NameGroup>();
        const wfChildren = groupSubagentsByWorkflow([
            mk({ agent_id: "a1", workflow_id: "shared-id" }),
            mk({ agent_id: "a2", workflow_id: "shared-id" }),
        ]);
        const nameChildren = groupSubagentsByWorkflow([
            mk({ agent_id: "b1", workflow_id: null, display_name: "shared-id" }),
            mk({ agent_id: "b2", workflow_id: null, display_name: "shared-id" }),
        ]);
        stabilizeGroupIdentity(cache, wfChildren);
        stabilizeGroupIdentity(cache, nameChildren);
        expect(cache.size).toBe(2);
        expect(cache.has("wf:shared-id")).toBe(true);
        expect(cache.has("name:block-1:shared-id")).toBe(true);
    });

    it("keeps same-named NameGroups from two different agent blocks as distinct cache entries (reagent P1 on #2123)", () => {
        // groupIdentityCache/expandedIds are shared across the WHOLE tree
        // (buildTree() calls groupSubagentsByWorkflow once per block, into
        // one cache) — a Haiku-generated name like "Code Reviewer" can
        // plausibly repeat across two unrelated agent panes. Without
        // parentBlockId in the key, block A's and block B's same-named
        // groups would stomp each other's identity/expand state.
        const cache = new Map<string, WorkflowGroup | NameGroup>();
        const blockAChildren = groupSubagentsByWorkflow([
            mk({ agent_id: "a1", parent_block_id: "block-A", workflow_id: null, display_name: "Code Reviewer" }),
            mk({ agent_id: "a2", parent_block_id: "block-A", workflow_id: null, display_name: "Code Reviewer" }),
        ]);
        const blockBChildren = groupSubagentsByWorkflow([
            mk({ agent_id: "b1", parent_block_id: "block-B", workflow_id: null, display_name: "Code Reviewer" }),
            mk({ agent_id: "b2", parent_block_id: "block-B", workflow_id: null, display_name: "Code Reviewer" }),
        ]);
        const stabilizedA = stabilizeGroupIdentity(cache, blockAChildren);
        const stabilizedB = stabilizeGroupIdentity(cache, blockBChildren);
        expect(cache.size).toBe(2);
        expect(stabilizedA[0]).not.toBe(stabilizedB[0]);
        if (isNameGroup(stabilizedA[0]) && isNameGroup(stabilizedB[0])) {
            expect(stabilizedA[0].subagents.map((s) => s.agent_id)).toEqual(["a1", "a2"]);
            expect(stabilizedB[0].subagents.map((s) => s.agent_id)).toEqual(["b1", "b2"]);
        } else {
            throw new Error("expected two name groups");
        }
    });
});

describe("groupCacheKey", () => {
    it("namespaces a WorkflowGroup key with 'wf:' and a NameGroup key with 'name:<parentBlockId>:'", () => {
        const [wf] = groupSubagentsByWorkflow([mk({ agent_id: "a1", workflow_id: "wf_1" })]).filter(isWorkflowGroup);
        const [ng] = groupSubagentsByWorkflow([
            mk({ agent_id: "a1", workflow_id: null, display_name: "N" }),
            mk({ agent_id: "a2", workflow_id: null, display_name: "N" }),
        ]).filter(isNameGroup);
        expect(groupCacheKey(wf)).toBe("wf:wf_1");
        expect(groupCacheKey(ng)).toBe("name:block-1:N");
    });
});

describe("pruneGroupIdentityCache", () => {
    it("drops entries for workflows no longer live, keeps the rest", () => {
        const cache = new Map<string, WorkflowGroup>();
        const groupA = groupSubagentsByWorkflow([mk({ agent_id: "a1", workflow_id: "wf_a" })]);
        const groupB = groupSubagentsByWorkflow([mk({ agent_id: "b1", workflow_id: "wf_b" })]);
        stabilizeGroupIdentity(cache, groupA);
        stabilizeGroupIdentity(cache, groupB);
        expect(cache.size).toBe(2);

        // Live keys use groupCacheKey's namespaced "wf:<id>" form, not the
        // raw workflowId.
        pruneGroupIdentityCache(cache, new Set(["wf:wf_a"]));
        expect(cache.size).toBe(1);
        expect(cache.has("wf:wf_a")).toBe(true);
        expect(cache.has("wf:wf_b")).toBe(false);
    });

    it("drops entries for name groups no longer live, keeps the rest", () => {
        const cache = new Map<string, WorkflowGroup | NameGroup>();
        const groupA = groupSubagentsByWorkflow([
            mk({ agent_id: "a1", workflow_id: null, display_name: "A" }),
            mk({ agent_id: "a2", workflow_id: null, display_name: "A" }),
        ]);
        const groupB = groupSubagentsByWorkflow([
            mk({ agent_id: "b1", workflow_id: null, display_name: "B" }),
            mk({ agent_id: "b2", workflow_id: null, display_name: "B" }),
        ]);
        stabilizeGroupIdentity(cache, groupA);
        stabilizeGroupIdentity(cache, groupB);
        expect(cache.size).toBe(2);

        pruneGroupIdentityCache(cache, new Set(["name:block-1:A"]));
        expect(cache.size).toBe(1);
        expect(cache.has("name:block-1:A")).toBe(true);
        expect(cache.has("name:block-1:B")).toBe(false);
    });
});
