// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { ActiveSubagent } from "../../swarm/swarm-model";
import { subagentActivities, subagentToActivity } from "./subagent-adapter";

function mk(overrides: Partial<ActiveSubagent> & Pick<ActiveSubagent, "agent_id">): ActiveSubagent {
    return {
        slug: "some-slug",
        parent_agent: "parent",
        parent_block_id: "block-1",
        session_id: "session-1",
        status: "active",
        spawned_at: 1000,
        last_event_at: 1000,
        event_count: 0,
        model: null,
        workflow_id: null,
        display_name: null,
        ...overrides,
    };
}

describe("subagentToActivity", () => {
    it("maps active → running, using startedAt from spawned_at", () => {
        const a = subagentToActivity(mk({ agent_id: "a1", status: "active", spawned_at: 500 }));
        expect(a.kind).toBe("subagent");
        expect(a.status).toBe("running");
        expect(a.startedAt).toBe(500);
        expect(a.endedAt).toBeUndefined();
    });

    it("maps completed → done, using last_event_at as endedAt", () => {
        const a = subagentToActivity(mk({ agent_id: "a1", status: "completed", last_event_at: 900 }));
        expect(a.status).toBe("done");
        expect(a.endedAt).toBe(900);
    });

    it("prefers display_name, falling back to slug then agent_id", () => {
        expect(subagentToActivity(mk({ agent_id: "a1", display_name: "Fixing auth" })).title).toBe("Fixing auth");
        expect(subagentToActivity(mk({ agent_id: "a1", display_name: null, slug: "cool-slug" })).title).toBe("cool-slug");
        expect(subagentToActivity(mk({ agent_id: "a1", display_name: null, slug: "" })).title).toBe("a1");
    });

    it("is never stoppable — no cancel RPC exists for subagents today", () => {
        expect(subagentToActivity(mk({ agent_id: "a1", status: "active" })).canStop).toBe(false);
    });

    it("carries the source ActiveSubagent for the row's tail + expanded view", () => {
        const sub = mk({ agent_id: "a1" });
        expect(subagentToActivity(sub).subagent).toBe(sub);
    });
});

describe("subagentActivities", () => {
    it("filters to only the given block's subagents", () => {
        const all = [
            mk({ agent_id: "a1", parent_block_id: "block-1" }),
            mk({ agent_id: "a2", parent_block_id: "block-2" }),
            mk({ agent_id: "a3", parent_block_id: "block-1" }),
        ];
        const result = subagentActivities(all, "block-1");
        expect(result.map((a) => a.id)).toEqual(["a1", "a3"]);
    });

    it("returns an empty array when no subagents match the block", () => {
        expect(subagentActivities([mk({ agent_id: "a1", parent_block_id: "block-2" })], "block-1")).toEqual([]);
    });

    it("collapses a Workflow-tool batch (shared workflow_id) into one row, not one per subagent", () => {
        // Regression for the live-reported "dozens of subagents in the dock
        // for 1-2 tool calls" bug — a single Workflow-tool call spawning many
        // subagents must render as one dock row, matching the Swarm tree's
        // own WorkflowGroup collapse.
        const all = [
            mk({ agent_id: "a1", workflow_id: "wf_1", slug: "batch", spawned_at: 100, last_event_at: 300, status: "active" }),
            mk({ agent_id: "a2", workflow_id: "wf_1", slug: "batch", spawned_at: 150, last_event_at: 400, status: "active" }),
            mk({ agent_id: "a3", workflow_id: "wf_1", slug: "batch", spawned_at: 200, last_event_at: 500, status: "completed" }),
        ];
        const result = subagentActivities(all, "block-1");
        expect(result).toHaveLength(1);
        expect(result[0].id).toBe("wf:wf_1");
        expect(result[0].title).toBe("batch (3)");
        expect(result[0].status).toBe("running"); // ≥1 member still active
        expect(result[0].startedAt).toBe(100); // earliest member
        expect(result[0].canStop).toBe(false);
    });

    it("marks a workflow group done once every member is terminal", () => {
        const all = [
            mk({ agent_id: "a1", workflow_id: "wf_1", slug: "batch", status: "completed", last_event_at: 300 }),
            mk({ agent_id: "a2", workflow_id: "wf_1", slug: "batch", status: "abandoned", last_event_at: 400 }),
        ];
        const result = subagentActivities(all, "block-1");
        expect(result).toHaveLength(1);
        expect(result[0].status).toBe("done");
        expect(result[0].endedAt).toBe(400);
    });

    it("collapses same-name loose subagents (no workflow_id) into one row", () => {
        const all = [
            mk({ agent_id: "a1", display_name: "Code Reviewer", spawned_at: 100 }),
            mk({ agent_id: "a2", display_name: "Code Reviewer", spawned_at: 200 }),
        ];
        const result = subagentActivities(all, "block-1");
        expect(result).toHaveLength(1);
        expect(result[0].title).toBe("Code Reviewer (2)");
    });

    it("leaves a lone, uniquely-named subagent as its own row", () => {
        const all = [mk({ agent_id: "a1", display_name: "One-off task" })];
        const result = subagentActivities(all, "block-1");
        expect(result).toHaveLength(1);
        expect(result[0].id).toBe("a1");
        expect(result[0].title).toBe("One-off task");
    });
});
