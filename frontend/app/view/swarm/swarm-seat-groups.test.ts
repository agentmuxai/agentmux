// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Group by lead (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4): leaders under
 * the operator, workers under their lead, a lead with no row heading its own
 * group, unseated agents under "No seat", the given order kept in each group.
 */

import { describe, expect, it } from "vitest";

import type { SeatSummary } from "@/types/rpc/SeatSummary";
import { groupRowsByLead, readGroupBy, type SeatListEntry, type SeatRowInfo } from "./swarm-seat-groups";

const seat = (role: SeatSummary["role"], lead?: [string, string | null]): SeatSummary => ({
    role,
    qualifier: null,
    lead: lead ? { id: lead[0], name: lead[1] } : null,
    area: null,
});
const local = (agentId: string | null, s: SeatSummary | null): SeatRowInfo => ({ agentId, seat: s, remote: false });

/** The entries as short strings: "# heading (count)" and "  key" indented by depth. */
const show = (entries: SeatListEntry[]) =>
    entries.map((e) => (e.kind === "heading" ? `# ${e.label} (${e.count})` : `${"  ".repeat(e.depth)}${e.key}`));

describe("groupRowsByLead", () => {
    it("puts leaders under the operator and each worker under its lead, unseated agents under No seat", () => {
        const info = new Map<string, SeatRowInfo>([
            ["w1", local("uid-w1", seat("worker", ["uid-camper", "Camper"]))],
            ["free", local("uid-free", null)],
            ["camper", local("uid-camper", seat("leader"))],
            ["opaz", local("uid-opaz", seat("leader"))],
            ["w2", local("uid-w2", seat("worker", ["uid-opaz", "Opaz"]))],
            ["w3", local("uid-w3", seat("worker", ["uid-camper", "Camper"]))],
        ]);
        const order = ["w1", "free", "camper", "opaz", "w2", "w3"];
        expect(show(groupRowsByLead(order, info))).toEqual([
            "# The operator (5)",
            "camper",
            "  w1",
            "  w3",
            "opaz",
            "  w2",
            "# No seat (1)",
            "free",
        ]);
    });

    it("keeps the given order among siblings and never drops a row", () => {
        const info = new Map<string, SeatRowInfo>([
            ["b", local("uid-b", seat("specialist"))],
            ["a", local("uid-a", seat("leader"))],
            ["x", local(null, null)],
            ["y", local("uid-y", null)],
        ]);
        const entries = groupRowsByLead(["x", "b", "y", "a"], info);
        expect(show(entries)).toEqual(["# The operator (2)", "b", "a", "# No seat (2)", "x", "y"]);
    });

    it("gives a lead with no row here its own group, by its name", () => {
        const info = new Map<string, SeatRowInfo>([
            ["w", local("uid-w", seat("worker", ["uid-gone", "Korp"]))],
            ["far", local("uid-far", seat("worker", ["uid-remote", null]))],
            ["lead", local("uid-lead", seat("leader"))],
        ]);
        expect(show(groupRowsByLead(["w", "far", "lead"], info))).toEqual([
            "# The operator (1)",
            "lead",
            "# Korp (1)",
            "w",
            "# an agent on another machine (1)",
            "far",
        ]);
    });

    it("nests a worker's own reports one step further", () => {
        const info = new Map<string, SeatRowInfo>([
            ["sub", local("uid-sub", seat("worker", ["uid-w", "W"]))],
            ["w", local("uid-w", seat("worker", ["uid-l", "L"]))],
            ["l", local("uid-l", seat("leader"))],
        ]);
        expect(show(groupRowsByLead(["sub", "w", "l"], info))).toEqual(["# The operator (3)", "l", "  w", "    sub"]);
    });

    it("heads an agent's reports once when it has two panes", () => {
        const info = new Map<string, SeatRowInfo>([
            ["l1", local("uid-l", seat("leader"))],
            ["l2", local("uid-l", seat("leader"))],
            ["w", local("uid-w", seat("worker", ["uid-l", "L"]))],
        ]);
        expect(show(groupRowsByLead(["l1", "l2", "w"], info))).toEqual(["# The operator (3)", "l1", "  w", "l2"]);
    });

    it("still shows agents caught in a cycle, under the operator", () => {
        const info = new Map<string, SeatRowInfo>([
            ["a", local("uid-a", seat("worker", ["uid-b", "B"]))],
            ["b", local("uid-b", seat("worker", ["uid-a", "A"]))],
        ]);
        expect(show(groupRowsByLead(["a", "b"], info))).toEqual(["# The operator (2)", "a", "  b"]);
    });

    it("puts other instances' agents under their own heading, and an unknown key under No seat", () => {
        const info = new Map<string, SeatRowInfo>([
            ["r1", { agentId: null, seat: null, remote: true }],
            ["l", local("uid-l", seat("leader"))],
        ]);
        expect(show(groupRowsByLead(["r1", "ghost", "l"], info))).toEqual([
            "# The operator (1)",
            "l",
            "# No seat (1)",
            "ghost",
            "# Other instances (1)",
            "r1",
        ]);
    });
});

describe("readGroupBy", () => {
    it("reads the pane's choice, none by default", () => {
        expect(readGroupBy({ "swarm:groupby": "lead" })).toBe("lead");
        expect(readGroupBy({ "swarm:groupby": "lane" })).toBe("none");
        expect(readGroupBy(undefined)).toBe("none");
    });
});
