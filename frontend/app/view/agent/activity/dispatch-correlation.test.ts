// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { ActiveSubagent, AgentDispatch } from "../../swarm/swarm-model";
import type { DocumentNode, ToolNode } from "../types";
import { correlateDispatchesForBlock } from "./dispatch-correlation";

function mkSub(overrides: Partial<ActiveSubagent> & Pick<ActiveSubagent, "agent_id" | "dispatch_id">): ActiveSubagent {
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
});
