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
    // THE regression test — reagent P1: a prior attempt at closing the
    // same-kind-parallel-spawn gap (a same-category COUNT cap) broke this
    // exact case, the overwhelmingly common one: a pane that has simply
    // made more than one Agent-tool call over the course of a session,
    // sequentially, not concurrently. Every one of these must still match.
    it("matches many SEQUENTIAL same-category calls across a whole session — the common case, not the rare parallel-spawn one", () => {
        const nodes: DocumentNode[] = [
            mkToolNode("tu_1", "Agent"),
            mkToolNode("tu_2", "Task"),
            mkToolNode("tu_3", "Agent"),
            mkToolNode("tu_4", "Agent"),
        ];
        const subagents = [
            mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 1_000 }),
            mkSub({ agent_id: "a2", dispatch_id: "solo:a2", spawned_at: 60_000 }),
            mkSub({ agent_id: "a3", dispatch_id: "solo:a3", spawned_at: 120_000 }),
            mkSub({ agent_id: "a4", dispatch_id: "solo:a4", spawned_at: 180_000 }),
        ];
        const dispatches = [
            mkDispatch({ dispatch_id: "solo:a1", dispatch_name: "First" }),
            mkDispatch({ dispatch_id: "solo:a2", dispatch_name: "Second" }),
            mkDispatch({ dispatch_id: "solo:a3", dispatch_name: "Third" }),
            mkDispatch({ dispatch_id: "solo:a4", dispatch_name: "Fourth" }),
        ];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.get("tu_1")?.dispatch_name).toBe("First");
        expect(result.get("tu_2")?.dispatch_name).toBe("Second");
        expect(result.get("tu_3")?.dispatch_name).toBe("Third");
        expect(result.get("tu_4")?.dispatch_name).toBe("Fourth");
    });

    it("returns an exact 1:1 match when counts and order line up", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1"), mkToolNode("tu_2")];
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
        const nodes: DocumentNode[] = [mkToolNode("tu_1"), mkToolNode("tu_2")];
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
        const nodes: DocumentNode[] = [mkToolNode("tu_1"), mkToolNode("tu_2")];
        const subagents = [mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 })];
        const dispatches = [mkDispatch({ dispatch_id: "solo:a1" })];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });

    it("falls back to an empty map when a dispatch has no orderable member (aged out of ListActive)", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1"), mkToolNode("tu_2")];
        // Only one of the two dispatches has any subagent data to order by.
        const subagents = [mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 })];
        const dispatches = [mkDispatch({ dispatch_id: "solo:a1" }), mkDispatch({ dispatch_id: "solo:a2" })];

        const result = correlateDispatchesForBlock("block-1", nodes, subagents, dispatches);

        expect(result.size).toBe(0);
    });

    // The one residual gap this file's design explicitly, honestly accepts
    // (see the module doc comment's history) — distinct, non-tied spawn
    // times from a genuine parallel same-turn spawn can still be
    // mismatched. This test instead exercises the guard this file DOES
    // provide: an exact millisecond tie, which is the unstable-sort case,
    // not the accepted residual gap.
    it("falls back to an empty map when two dispatches share the exact same spawned_at (tie guard)", () => {
        const nodes: DocumentNode[] = [mkToolNode("tu_1"), mkToolNode("tu_2")];
        const subagents = [
            mkSub({ agent_id: "a1", dispatch_id: "solo:a1", spawned_at: 100 }),
            mkSub({ agent_id: "a2", dispatch_id: "solo:a2", spawned_at: 100 }),
        ];
        const dispatches = [mkDispatch({ dispatch_id: "solo:a1" }), mkDispatch({ dispatch_id: "solo:a2" })];

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

    // Count equality alone doesn't guarantee a CORRECT pairing. A Task call
    // and a Workflow call spawned in the same turn can have counts line up
    // while their spawn-order position doesn't match their transcript-order
    // position, silently swapping which node gets which dispatch's kind.
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

    it("matches a mixed Agent/Task + Workflow spawn correctly when transcript order DOES agree with spawn order", () => {
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
});
