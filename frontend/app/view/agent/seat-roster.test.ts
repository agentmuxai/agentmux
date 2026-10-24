// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The roster import: parse, resolve by uid or exact name (never guessed), and order leads first. */

import { describe, expect, it } from "vitest";

import { applyInPasses, orderForApply, parseRoster, resolveRoster, type ResolvedSeat, type RosterAgent } from "./seat-roster";

const AGENTS: RosterAgent[] = [
    { id: "uid-x", name: "AgentX" },
    { id: "uid-c", name: "Camper" },
    { id: "uid-o", name: "Opaz" },
    { id: "uid-t1", name: "Twin" },
    { id: "uid-t2", name: "Twin" },
];

describe("parseRoster", () => {
    it("takes an object of agent → seat, or a list of seats", () => {
        expect(parseRoster('{"AgentX": {"role": "leader", "lead": null}}')).toEqual([
            { agent: "AgentX", role: "leader", qualifier: undefined, lead: null, area: undefined },
        ]);
        expect(parseRoster('[{"agent": "Camper", "role": "worker", "lead": "AgentX", "area": "panes"}]')).toEqual([
            { agent: "Camper", role: "worker", qualifier: undefined, lead: "AgentX", area: "panes" },
        ]);
    });

    it("says what is wrong with something that isn't a roster", () => {
        expect(() => parseRoster("{nope")).toThrow(/Not valid JSON/);
        expect(() => parseRoster('"leader"')).toThrow(/object of agent/);
        expect(() => parseRoster('{"AgentX": "leader"}')).toThrow(/"AgentX" isn't an object/);
        expect(() => parseRoster('[{"role": "worker"}]')).toThrow(/Line 1 doesn't name an agent/);
    });
});

describe("resolveRoster", () => {
    it("resolves agents and leads by uid or exact name; the operator is no lead", () => {
        const plan = resolveRoster(
            parseRoster(
                JSON.stringify({
                    AgentX: { role: "leader", qualifier: " product ", lead: "operator", area: "agent pane" },
                    "uid-c": { role: "worker", lead: "AgentX", area: "panes, layout, tabs" },
                })
            ),
            AGENTS
        );
        expect(plan.unresolved).toEqual([]);
        expect(plan.seats).toEqual([
            {
                ref: "AgentX",
                agent: { id: "uid-x", name: "AgentX" },
                seat: { role: "leader", qualifier: "product", lead: undefined, area: "agent pane" },
                leadName: "the operator",
            },
            {
                ref: "uid-c",
                agent: { id: "uid-c", name: "Camper" },
                seat: { role: "worker", qualifier: undefined, lead: "uid-x", area: "panes, layout, tabs" },
                leadName: "AgentX",
            },
        ]);
    });

    it("lists what it can't resolve, with why, and guesses nothing", () => {
        const plan = resolveRoster(
            parseRoster(
                JSON.stringify([
                    { agent: "agentx", role: "leader" },
                    { agent: "Twin", role: "worker" },
                    { agent: "Opaz", role: "worker", lead: "Nobody" },
                    { agent: "Camper", role: "boss" },
                    { agent: "AgentX", role: "leader", lead: "AgentX" },
                    { agent: "uid-c", role: "worker", area: "x".repeat(121) },
                ])
            ),
            AGENTS
        );
        expect(plan.seats).toEqual([]);
        expect(plan.unresolved).toEqual([
            { ref: "agentx", reason: 'no agent is named "agentx" or has that uid' },
            { ref: "Twin", reason: '2 agents are named "Twin"' },
            { ref: "Opaz", reason: 'its lead: no agent is named "Nobody" or has that uid' },
            { ref: "Camper", reason: 'unknown role "boss"' },
            { ref: "AgentX", reason: "an agent can't be its own lead" },
            { ref: "uid-c", reason: "the area is over 120 characters" },
        ]);
    });

    it("takes an agent once", () => {
        const plan = resolveRoster(
            parseRoster('[{"agent": "AgentX", "role": "leader"}, {"agent": "uid-x", "role": "worker"}]'),
            AGENTS
        );
        expect(plan.seats.map((s) => s.ref)).toEqual(["AgentX"]);
        expect(plan.unresolved).toEqual([{ ref: "uid-x", reason: "AgentX is listed more than once" }]);
    });
});

describe("orderForApply", () => {
    it("sets a lead's seat before its reports'", () => {
        const plan = resolveRoster(
            parseRoster(
                JSON.stringify({
                    Opaz: { role: "worker", lead: "Camper" },
                    Camper: { role: "worker", lead: "AgentX" },
                    AgentX: { role: "leader" },
                })
            ),
            AGENTS
        );
        expect(orderForApply(plan.seats).map((s) => s.agent.name)).toEqual(["AgentX", "Camper", "Opaz"]);
    });

    it("ends on a loop inside the roster rather than spinning", () => {
        const plan = resolveRoster(
            parseRoster(JSON.stringify({ AgentX: { role: "worker", lead: "Camper" }, Camper: { role: "worker", lead: "AgentX" } })),
            AGENTS
        );
        expect(orderForApply(plan.seats)).toHaveLength(2);
    });
});

describe("applyInPasses", () => {
    const line = (id: string, lead: string): ResolvedSeat => ({
        ref: id,
        agent: { id, name: id },
        seat: { role: "worker", lead },
        leadName: lead,
    });

    /** A store of leads that refuses a lead whose chain comes back, as srv does. */
    const leads = (initial: Record<string, string>) => {
        const lead = { ...initial };
        const set = async (s: ResolvedSeat) => {
            for (let at: string | undefined = s.seat.lead ?? undefined; at; at = lead[at]) {
                if (at === s.agent.id) throw new Error(`INVALID_ARG: ${s.seat.lead} already reports to this agent (a cycle)`);
            }
            lead[s.agent.id] = s.seat.lead as string;
        };
        return { lead, set };
    };

    it("retries a line refused as a cycle once a later one lands", async () => {
        // X → A → B today; the roster moves B under X and A under C.
        const store = leads({ X: "A", A: "B" });
        const out = await applyInPasses(orderForApply([line("B", "X"), line("A", "C")]), store.set);
        expect(out.map((o) => [o.seat.agent.id, o.error])).toEqual([
            ["B", null],
            ["A", null],
        ]);
        expect(store.lead).toEqual({ X: "A", A: "C", B: "X" });
    });

    it("stops when a pass lands nothing, keeping each refusal", async () => {
        const store = leads({ A: "B" });
        const out = await applyInPasses([line("B", "A"), line("C", "nobody")], async (s) => {
            if (s.agent.id === "C") throw new Error("lead nobody is not an agent");
            await store.set(s);
        });
        expect(out[0].error).toMatch(/cycle/);
        expect(out[1].error).toMatch(/not an agent/);
    });
});
