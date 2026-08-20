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

// `timestamp` defaults to `undefined` — deliberately, NOT a shared literal.
// correlateDispatchesForBlock's same-turn guard treats a missing timestamp
// as unverifiable (bail), so any test with 2+ same-category dispatch-kind
// nodes that expects a SUCCESSFUL match must pass explicit, well-separated
// timestamps (>= SAME_TURN_THRESHOLD_MS apart). A test with only one
// dispatch-kind node, or one deliberately exercising the same-turn guard
// itself, can omit it.
function mkToolNode(id: string, tool: ToolNode["tool"] = "Agent", timestamp?: number): ToolNode {
    return { type: "tool", id, tool, timestamp } as unknown as ToolNode;
}

describe("correlateDispatchesForBlock", () => {
    it("returns an exact 1:1 match when counts and order line up", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Agent", 0), mkToolNode("tu_2", "Agent", 10_000)];
        const subagents = [
            mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 }),
            mkSub({ agent_id: "a2", dispatch_id: "solo:a2", spawned_at: 200 }),
        ];
        const dispatches = [
            mkDispatch({ dispatch_id: "solo:a1", dispatch_name: "First" }),
            mkDispatch({ dispatch_id: "solo:a2", dispatch_name: "Second" }),
        ];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.get("tu_1")?.dispatch_name).toBe("First");
        expect(result.get("tu_2")?.dispatch_name).toBe("Second");
    });

    it("orders by spawn time, not by ListDispatches' own (last_event_at-sorted) order", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Agent", 0), mkToolNode("tu_2", "Agent", 10_000)];
        const subagents = [
            mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 }),
            mkSub({ agent_id: "a2", dispatch_id: "solo:a2", spawned_at: 200 }),
        ];
        // Deliberately passed in reverse-spawn order, as ListDispatches would
        // return them (most-recently-active first).
        const dispatches = [
            mkDispatch({ dispatch_id: "solo:a2", dispatch_name: "Second" }),
            mkDispatch({ dispatch_id: "solo:a1", dispatch_name: "First" }),
        ];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.get("tu_1")?.dispatch_name).toBe("First");
        expect(result.get("tu_2")?.dispatch_name).toBe("Second");
    });

    it("falls back to an empty map on a tool-node/dispatch count mismatch", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Agent", 0), mkToolNode("tu_2", "Agent", 10_000)];
        const subagents = [mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 })];
        const dispatches = [mkDispatch({ dispatch_id: "solo:a1" })];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });

    it("falls back to an empty map when a dispatch has no orderable member (aged out of ListActive)", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Agent", 0), mkToolNode("tu_2", "Agent", 10_000)];
        // Only one of the two dispatches has any subagent data to order by.
        const subagents = [mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 })];
        const dispatches = [mkDispatch({ dispatch_id: "solo:a1" }), mkDispatch({ dispatch_id: "solo:a2" })];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });

    // Reagent/codex P1 (PR #2676 review): two dispatches spawned within the
    // same millisecond produce an unstable sort — bail rather than trust an
    // arbitrary tiebreak, since this could silently swap two same-kind
    // calls (which the kind-compatibility check can't catch either). Node
    // timestamps are deliberately far apart so the SAME-TURN guard doesn't
    // also fire here — this test is specifically about the spawned_at tie.
    it("falls back to an empty map when two dispatches share the exact same spawned_at (tie guard)", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Agent", 0), mkToolNode("tu_2", "Agent", 10_000)];
        const subagents = [
            mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 }),
            mkSub({ agent_id: "a2", dispatch_id: "solo:a2", spawned_at: 100 }),
        ];
        const dispatches = [mkDispatch({ dispatch_id: "solo:a1" }), mkDispatch({ dispatch_id: "solo:a2" })];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });

    // Same-turn ambiguity guard (the ACTUAL fix, after two prior attempts —
    // a slug-based guard was flagged by reagent as not covering this exact
    // scenario: two separate solo Agent-tool calls, each with its own
    // unique dispatch_id/slug, issued in one turn). Distinct, non-tied
    // spawned_at + matching kinds still isn't enough; if the transcript's
    // own tool_use timestamps are within SAME_TURN_THRESHOLD_MS of each
    // other, treat the pair as unverifiably ordered.
    it("falls back to an empty map when two same-category tool nodes' own timestamps are within the same-turn threshold, even with distinct non-tied spawned_at and matching kinds", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Agent", 5000), mkToolNode("tu_2", "Agent", 5001)];
        const subagents = [
            mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 }),
            mkSub({ agent_id: "a2", dispatch_id: "solo:a2", spawned_at: 200 }),
        ];
        const dispatches = [mkDispatch({ dispatch_id: "solo:a1" }), mkDispatch({ dispatch_id: "solo:a2" })];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });

    it("falls back to an empty map when either same-category tool node is missing a timestamp", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Agent", undefined), mkToolNode("tu_2", "Agent", 10_000)];
        const subagents = [
            mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 }),
            mkSub({ agent_id: "a2", dispatch_id: "solo:a2", spawned_at: 200 }),
        ];
        const dispatches = [mkDispatch({ dispatch_id: "solo:a1" }), mkDispatch({ dispatch_id: "solo:a2" })];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });

    it("matches two same-category tool nodes whose own timestamps are far enough apart to be distinct turns", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Agent", 0), mkToolNode("tu_2", "Agent", 10_000)];
        const subagents = [
            mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 }),
            mkSub({ agent_id: "a2", dispatch_id: "solo:a2", spawned_at: 200 }),
        ];
        const dispatches = [
            mkDispatch({ dispatch_id: "solo:a1", dispatch_name: "First" }),
            mkDispatch({ dispatch_id: "solo:a2", dispatch_name: "Second" }),
        ];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.get("tu_1")?.dispatch_name).toBe("First");
        expect(result.get("tu_2")?.dispatch_name).toBe("Second");
    });

    it("falls back to an empty map when more than one Workflow tool node's timestamps are within the same-turn threshold", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Workflow", 0), mkToolNode("tu_2", "Workflow", 1)];
        const subagents = [
            mkSub({ agent_id: "m1", dispatch_id: "wf_1", spawned_at: 100 }),
            mkSub({ agent_id: "m2", dispatch_id: "wf_2", spawned_at: 200 }),
        ];
        const dispatches = [
            mkDispatch({ dispatch_id: "wf_1", kind: "workflow" }),
            mkDispatch({ dispatch_id: "wf_2", kind: "workflow" }),
        ];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });

    it("matches two Workflow tool nodes far enough apart to be distinct turns", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1", "Workflow", 0), mkToolNode("tu_2", "Workflow", 10_000)];
        const subagents = [
            mkSub({ agent_id: "m1", dispatch_id: "wf_1", spawned_at: 100 }),
            mkSub({ agent_id: "m2", dispatch_id: "wf_2", spawned_at: 200 }),
        ];
        const dispatches = [
            mkDispatch({ dispatch_id: "wf_1", kind: "workflow", dispatch_name: "First run" }),
            mkDispatch({ dispatch_id: "wf_2", kind: "workflow", dispatch_name: "Second run" }),
        ];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.get("tu_1")?.dispatch_name).toBe("First run");
        expect(result.get("tu_2")?.dispatch_name).toBe("Second run");
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
    // gets which dispatch's kind. Timestamps are far apart (different
    // categories anyway, so the same-turn guard wouldn't fire regardless).
    it("falls back to an empty map when a mixed Agent/Task + Workflow spawn's transcript order disagrees with spawn order (kind-mismatch guard)", () => {
        // Transcript order: Task node first, Workflow node second.
        const nodes: DocumentNode[] = [mkToolNode("tu_task", "Task", 0), mkToolNode("tu_wf", "Workflow", 10_000)];
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

    it("matches a mixed Agent/Task + Workflow spawn correctly when transcript order DOES agree with spawn order", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_task", "Task", 0), mkToolNode("tu_wf", "Workflow", 10_000)];
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
});
