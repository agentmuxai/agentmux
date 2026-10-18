// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { HistoryAgent } from "@/types/rpc/HistoryAgent";
import { agentRows, historyTotals } from "./warden-history-shared";

function agent(id: string, extra: Partial<HistoryAgent> = {}): HistoryAgent {
    return {
        definition_id: id,
        name: id.toUpperCase(),
        provider: "claude",
        fork_of: "",
        fork_of_name: "",
        deleted: false,
        current_bytes: 100,
        archives: 0,
        archive_bytes: 0,
        last_active_ms: 0,
        ...extra,
    };
}

const ids = (rows: ReturnType<typeof agentRows>) => rows.map((r) => `${"  ".repeat(r.depth)}${r.agent.definition_id}`);

describe("agentRows", () => {
    it("puts each fork under the agent it was forked from, keeping the given order", () => {
        const agents = [agent("b"), agent("a2", { fork_of: "a" }), agent("a"), agent("a2x", { fork_of: "a2" })];
        expect(ids(agentRows(agents, ""))).toEqual(["b", "a", "  a2", "    a2x"]);
    });

    it("a fork whose parent has no history is a root", () => {
        expect(ids(agentRows([agent("f", { fork_of: "gone" })], ""))).toEqual(["f"]);
    });

    it("forks that point at each other are still listed once", () => {
        const agents = [agent("x", { fork_of: "y" }), agent("y", { fork_of: "x" })];
        expect(ids(agentRows(agents, "")).map((s) => s.trim()).sort()).toEqual(["x", "y"]);
    });

    it("a filter lists the matching agents flat, by name, id, provider or parent", () => {
        const agents = [agent("a"), agent("a2", { fork_of: "a", fork_of_name: "Alpha" }), agent("c", { provider: "codex" })];
        expect(ids(agentRows(agents, "alpha"))).toEqual(["a2"]);
        expect(ids(agentRows(agents, "CODEX"))).toEqual(["c"]);
        expect(ids(agentRows(agents, "a2"))).toEqual(["a2"]);
    });
});

describe("historyTotals", () => {
    it("adds current and archived history and counts archives", () => {
        const t = historyTotals([agent("a", { current_bytes: 10, archive_bytes: 5, archives: 2 }), agent("b", { current_bytes: 1 })]);
        expect(t).toEqual({ agents: 2, bytes: 16, archives: 2 });
    });
});
