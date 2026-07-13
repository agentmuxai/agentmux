// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { subagentDisplayStatus } from "./swarm-view";
import type { ActiveSubagent } from "./swarm-model";

function mk(overrides: Partial<ActiveSubagent> & Pick<ActiveSubagent, "agent_id">): ActiveSubagent {
    return {
        slug: "",
        parent_agent: "parent",
        parent_block_id: "block-1",
        session_id: "session-1",
        status: "active",
        spawned_at: 0,
        last_event_at: 0,
        event_count: 1,
        model: null,
        workflow_id: null,
        display_name: null,
        ...overrides,
    };
}

describe("subagentDisplayStatus", () => {
    it("shows 'working' for an active subagent whose parent is running", () => {
        const sub = mk({ agent_id: "a1", status: "active" });
        expect(subagentDisplayStatus(sub, "running")).toBe("working");
    });

    it("shows 'idle' for a completed subagent, regardless of parent status", () => {
        const sub = mk({ agent_id: "a1", status: "completed" });
        expect(subagentDisplayStatus(sub, "running")).toBe("idle");
        expect(subagentDisplayStatus(sub, "idle")).toBe("idle");
    });

    it("shows 'interrupted' for a backend-confirmed abandoned subagent, regardless of parent status", () => {
        const sub = mk({ agent_id: "a1", status: "abandoned" });
        expect(subagentDisplayStatus(sub, "running")).toBe("interrupted");
        expect(subagentDisplayStatus(sub, "idle")).toBe("interrupted");
    });

    it("client-side backstop: shows 'interrupted' (not 'working') for a still-active subagent whose parent has already gone idle", () => {
        // A subagent cannot genuinely still be active once its parent's own
        // turn has ended (Task-tool calls are synchronous within the
        // parent's turn) — the backend hasn't reconciled this one yet
        // (still reports "active"), but the frontend has the same
        // parent-idle signal available for free and shouldn't keep
        // rendering it as "working".
        const sub = mk({ agent_id: "a1", status: "active" });
        expect(subagentDisplayStatus(sub, "idle")).toBe("interrupted");
    });

    it("never mutates the underlying subagent — it's a pure display projection", () => {
        const sub = mk({ agent_id: "a1", status: "active" });
        const before = { ...sub };
        subagentDisplayStatus(sub, "idle");
        expect(sub).toEqual(before);
    });
});
