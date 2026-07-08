// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { groupSubagentsByWorkflow, isWorkflowGroup, type ActiveSubagent } from "./swarm-model";

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
