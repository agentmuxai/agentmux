// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { ActiveSubagent, AgentDispatch } from "../../swarm/swarm-model";
import type { DocumentNode, ToolNode } from "../types";
import { correlateDispatchesForBlock } from "./dispatch-correlation";

function mkSub(overrides: Partial<ActiveSubagent> & Pick<ActiveSubagent, "agent_id" | "dispatch_id">): ActiveSubagent {
    return {
        slug: `slug-${overrides.agent_id}`,
        parent_agent: "parent",
        parent_block_id: "block-1",
        session_id: "session-1",
        status: "active",
        spawned_at: 1000,
        last_event_at: 1000,
        event_count: 0,
        model: null,
        display_name: null,
        ...overrides,
    };
}

function mkDispatch(overrides: Partial<AgentDispatch> & Pick<AgentDispatch, "dispatch_id">): AgentDispatch {
    return {
        kind: "solo",
        parent_agent: "parent",
        parent_block_id: "block-1",
        session_id: "session-1",
        member_count: 1,
        members_done: 0,
        status: "running",
        last_event_at: 1000,
        dispatch_name: null,
        ...overrides,
    } as AgentDispatch;
}

function mkToolNode(id: string, tool: ToolNode["tool"] = "Agent"): ToolNode {
    return { type: "tool", id, tool } as unknown as ToolNode;
}

describe("correlateDispatchesForBlock", () => {
    it("matches a single solo dispatch (the common, unambiguous case)", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Agent")];
        const subagents = [mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 })];
        const dispatches = [mkDispatch({ dispatch_id: "solo:a1", dispatch_name: "First" })];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.get("tu_1")?.dispatch_name).toBe("First");
    });

    it("matches a single Workflow dispatch (the common, unambiguous case)", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Workflow")];
        const subagents = [mkSub({ agent_id: "m1", dispatch_id: "wf_1", spawned_at: 100 })];
        const dispatches = [mkDispatch({ dispatch_id: "wf_1", kind: "workflow", dispatch_name: "Run" })];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.get("tu_1")?.dispatch_name).toBe("Run");
    });

    it("matches one solo AND one Workflow call together, correctly attributed by kind", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_task", "Task"), mkToolNode("tu_wf", "Workflow")];
        const subagents = [
            mkSub({ agent_id: "task-agent", dispatch_id: "solo:task-agent", spawned_at: 100 }),
            mkSub({ agent_id: "wf-member", dispatch_id: "wf_1", spawned_at: 200 }),
        ];
        const dispatches = [
            mkDispatch({ dispatch_id: "solo:task-agent", kind: "solo", dispatch_name: "Task dispatch" }),
            mkDispatch({ dispatch_id: "wf_1", kind: "workflow", dispatch_name: "Workflow dispatch" }),
        ];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.get("tu_task")?.dispatch_name).toBe("Task dispatch");
        expect(result.get("tu_wf")?.dispatch_name).toBe("Workflow dispatch");
    });

    // The core same-category-cap policy — see the module doc comment for
    // why this is now unconditional rather than an attempted heuristic
    // detection (two prior heuristics, a shared-slug check and a
    // ToolNode.timestamp-gap threshold, both failed review scrutiny across
    // five rounds). Two solo (or two Workflow) calls always bail now,
    // regardless of how far apart their spawned_at or transcript position
    // are — there is no longer any code path that could match them
    // incorrectly, because there is no code path that matches them at all.
    it("falls back to an empty map when a pane has two solo dispatch-kind tool nodes, no matter how far apart they are", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Agent"), mkToolNode("tu_2", "Task")];
        const subagents = [
            mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 }),
            mkSub({ agent_id: "a2", dispatch_id: "solo:a2", spawned_at: 999_999 }),
        ];
        const dispatches = [mkDispatch({ dispatch_id: "solo:a1" }), mkDispatch({ dispatch_id: "solo:a2" })];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });

    it("falls back to an empty map when a pane has two Workflow dispatch-kind tool nodes", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Workflow"), mkToolNode("tu_2", "Workflow")];
        const subagents = [
            mkSub({ agent_id: "m1", dispatch_id: "wf_1", spawned_at: 100 }),
            mkSub({ agent_id: "m2", dispatch_id: "wf_2", spawned_at: 999_999 }),
        ];
        const dispatches = [
            mkDispatch({ dispatch_id: "wf_1", kind: "workflow" }),
            mkDispatch({ dispatch_id: "wf_2", kind: "workflow" }),
        ];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });

    it("still matches the solo+Workflow pair even when three or more same-category calls would have been ambiguous elsewhere — the cap is per-category, not global", () => {
        // Sanity check that the per-category cap doesn't over-trigger: one
        // solo and one workflow call, each alone in its own category,
        // still match even though the PANE has 2 dispatch-kind nodes total.
        const nodes: DocumentNode[] = [mkToolNode("tu_task", "Agent"), mkToolNode("tu_wf", "Workflow")];
        const subagents = [
            mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 }),
            mkSub({ agent_id: "m1", dispatch_id: "wf_1", spawned_at: 200 }),
        ];
        const dispatches = [
            mkDispatch({ dispatch_id: "solo:a1", dispatch_name: "Solo" }),
            mkDispatch({ dispatch_id: "wf_1", kind: "workflow", dispatch_name: "Workflow" }),
        ];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.get("tu_task")?.dispatch_name).toBe("Solo");
        expect(result.get("tu_wf")?.dispatch_name).toBe("Workflow");
    });

    it("falls back to an empty map on a tool-node/dispatch count mismatch", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Agent")];
        const subagents: ActiveSubagent[] = [];
        const dispatches = [mkDispatch({ dispatch_id: "solo:a1" }), mkDispatch({ dispatch_id: "solo:a2" })];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });

    it("falls back to an empty map when a dispatch has no orderable member (aged out of ListActive)", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Agent")];
        const subagents: ActiveSubagent[] = []; // no member data to order by at all
        const dispatches = [mkDispatch({ dispatch_id: "solo:a1" })];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });

    it("ignores subagents/dispatches from other blocks", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1")];
        const subagents = [
            mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100, parent_block_id: "block-1" }),
            mkSub({ agent_id: "a9", dispatch_id: "solo:a9", spawned_at: 50, parent_block_id: "other-block" }),
        ];
        const dispatches = [
            mkDispatch({ dispatch_id: "solo:a1", parent_block_id: "block-1", dispatch_name: "Mine" }),
            mkDispatch({ dispatch_id: "solo:a9", parent_block_id: "other-block", dispatch_name: "NotMine" }),
        ];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.get("tu_1")?.dispatch_name).toBe("Mine");
        expect(result.size).toBe(1);
    });

    it("returns an empty map when there are no dispatch-kind tool nodes", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Bash")];
        const result = correlateDispatchesForBlock("block-1", nodes, [], []);
        expect(result.size).toBe(0);
    });

    // Reagent P1 (PR #2676 review): count equality alone doesn't guarantee a
    // CORRECT pairing. A Task call and a Workflow call spawned in the same
    // turn can have counts line up while their spawn-order position doesn't
    // match their transcript-order position, silently swapping which node
    // gets which dispatch's kind.
    it("falls back to an empty map when a mixed Agent/Task + Workflow spawn's transcript order disagrees with spawn order (kind-mismatch guard)", () => {
        // Transcript order: Task node first, Workflow node second.
        const nodes: DocumentNode[] = [mkToolNode("tu_task", "Task"), mkToolNode("tu_wf", "Workflow")];
        // Spawn order: the Workflow's first member (spawned_at 100) actually
        // started BEFORE the Task's own subagent (spawned_at 200) — the
        // opposite of transcript order.
        const subagents = [
            mkSub({ agent_id: "task-agent", dispatch_id: "solo:task-agent", spawned_at: 200 }),
            mkSub({ agent_id: "wf-member", dispatch_id: "wf_1", spawned_at: 100 }),
        ];
        const dispatches = [
            mkDispatch({ dispatch_id: "solo:task-agent", kind: "solo" }),
            mkDispatch({ dispatch_id: "wf_1", kind: "workflow" }),
        ];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });
});
