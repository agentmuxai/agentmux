// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    groupSubagentsByWorkflow,
    isWorkflowGroup,
    mergeSubagentsPreservingIdentity,
    pruneGroupIdentityCache,
    stabilizeGroupIdentity,
    type ActiveSubagent,
    type WorkflowGroup,
} from "./swarm-model";

function mk(overrides: Partial<ActiveSubagent> & Pick<ActiveSubagent, "agent_id">): ActiveSubagent {
    return {
        slug: "",
        parent_agent: "parent",
        parent_block_id: "block-1",
        session_id: "session-1",
        status: "completed",
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
        expect(isWorkflowGroup(result[0]) ? result[0].workflowId : result[0].agent_id).toBe("newest-loose");
        expect(isWorkflowGroup(result[2]) ? result[2].workflowId : result[2].agent_id).toBe("old-loose");
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
});

describe("pruneGroupIdentityCache", () => {
    it("drops entries for workflows no longer live, keeps the rest", () => {
        const cache = new Map<string, WorkflowGroup>();
        const groupA = groupSubagentsByWorkflow([mk({ agent_id: "a1", workflow_id: "wf_a" })]);
        const groupB = groupSubagentsByWorkflow([mk({ agent_id: "b1", workflow_id: "wf_b" })]);
        stabilizeGroupIdentity(cache, groupA);
        stabilizeGroupIdentity(cache, groupB);
        expect(cache.size).toBe(2);

        pruneGroupIdentityCache(cache, new Set(["wf_a"]));
        expect(cache.size).toBe(1);
        expect(cache.has("wf_a")).toBe(true);
        expect(cache.has("wf_b")).toBe(false);
    });
});
