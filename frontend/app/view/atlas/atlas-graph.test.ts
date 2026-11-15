// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { SeatSummary } from "@/types/rpc/SeatSummary";
import { buildAtlasGraph, type AtlasAgent, type AtlasMachine, type AtlasOrphan, type AtlasService, type AtlasState } from "./atlas-graph";

const machine = (key: string, title = key): AtlasMachine => ({
    key,
    title,
    badge: "LAN",
    platform: null,
    stale: false,
    seenAtMs: null,
});

const seat = (lead: SeatSummary["lead"], qualifier: string | null = null): SeatSummary => ({
    role: "worker",
    qualifier,
    lead,
    area: null,
});

function agent(name: string, m: string, over: Partial<AtlasAgent> = {}): AtlasAgent {
    return {
        key: `${m}/${name}`,
        name,
        provider: "claude",
        state: "idle" as AtlasState,
        line: null,
        machine: m,
        agentId: `id-${name}`,
        seat: null,
        contextTokens: null,
        currentTool: null,
        blockId: null,
        runtimeId: null,
        remote: false,
        stale: false,
        ...over,
    };
}

describe("buildAtlasGraph", () => {
    it("makes a region per machine with agents, and folds the stopped ones", () => {
        const g = buildAtlasGraph(
            [machine("a"), machine("b"), machine("c")],
            [agent("x", "a"), agent("y", "a", { state: "stopped" }), agent("z", "c")]
        );
        expect(g.regions.map((r) => r.key)).toEqual(["a", "c"]);
        expect(g.regions[0].shown.map((n) => n.name)).toEqual(["x"]);
        expect(g.regions[0].folded.map((n) => n.name)).toEqual(["y"]);
        expect(g.edges).toEqual([]);
    });

    it("draws a line from a lead to the agent that reports to it, by id", () => {
        const g = buildAtlasGraph(
            [machine("a")],
            [agent("boss", "a"), agent("w", "a", { seat: seat({ id: "id-boss", name: "boss" }) })]
        );
        expect(g.edges).toEqual([{ from: "a/boss", to: "a/w", kind: "lead" }]);
    });

    it("marks a spawned worker line", () => {
        const g = buildAtlasGraph(
            [machine("a")],
            [agent("boss", "a"), agent("w", "a", { seat: seat({ id: "id-boss", name: null }, "spawned") })]
        );
        expect(g.edges[0].kind).toBe("spawned");
    });

    it("follows a lead written Name@host to the machine it is on", () => {
        const g = buildAtlasGraph(
            [machine("a", "desk"), machine("b", "lab-1"), machine("c", "lab-2")],
            [
                agent("Lead", "a"),
                agent("Lead", "c", { agentId: "other" }),
                agent("w", "b", { remote: true, seat: seat({ id: "remote:b/lead", name: "Lead@lab-2" }) }),
            ]
        );
        expect(g.edges).toEqual([{ from: "c/Lead", to: "b/w", kind: "lead" }]);
    });

    it("follows a lead with the id of an agent of another instance within the same section", () => {
        const g = buildAtlasGraph(
            [machine("b")],
            [
                agent("Lead", "b", { agentId: "remote:b/lead", remote: true }),
                agent("w", "b", { remote: true, seat: seat({ id: "remote:b/lead", name: "Lead" }) }),
            ]
        );
        expect(g.edges).toEqual([{ from: "b/Lead", to: "b/w", kind: "lead" }]);
    });

    it("draws no line to a lead that is stopped, absent, or that two agents could be", () => {
        const g = buildAtlasGraph(
            [machine("a"), machine("b")],
            [
                agent("gone", "a", { state: "stopped" }),
                agent("w1", "a", { seat: seat({ id: "id-gone", name: "gone" }) }),
                agent("w2", "a", { seat: seat({ id: "nobody", name: "nobody" }) }),
                agent("twin", "a", { agentId: "t1" }),
                agent("twin", "b", { agentId: "t2" }),
                agent("w3", "b", { seat: seat({ id: "other-twin", name: "twin@elsewhere" }) }),
                agent("w4", "b", { seat: seat({ id: "x", name: null }) }),
            ]
        );
        expect(g.edges).toEqual([]);
    });

    it("takes a bare lead name on its own machine before any other", () => {
        const g = buildAtlasGraph(
            [machine("a"), machine("b")],
            [
                agent("Lead", "a", { agentId: "1" }),
                agent("Lead", "b", { agentId: "2" }),
                agent("w", "b", { seat: seat({ id: "zzz", name: "Lead" }) }),
            ]
        );
        expect(g.edges).toEqual([{ from: "b/Lead", to: "b/w", kind: "lead" }]);
    });

    it("never makes an agent its own lead", () => {
        const g = buildAtlasGraph([machine("a")], [agent("w", "a", { seat: seat({ id: "id-w", name: "w" }) })]);
        expect(g.edges).toEqual([]);
    });
});

describe("what is not an agent", () => {
    const orphan = (name: string, m: string): AtlasOrphan => ({
        key: `container:${name}`,
        machine: m,
        name,
        state: "running",
        image: "agentmux/agent:1",
        sinceMs: 1_000,
    });
    const service = (key: string): AtlasService => ({ key, name: key, icon: "robot", state: "ok", sub: "", lines: [] });

    it("puts a container no agent uses in its machine's region, and keeps a machine that has nothing else", () => {
        const g = buildAtlasGraph([machine("a"), machine("b"), machine("c")], [agent("x", "a")], {
            orphans: [orphan("agentmux-1", "a"), orphan("agentmux-2", "b")],
        });
        expect(g.regions.map((r) => `${r.key}: ${r.shown.length} agents, ${r.orphans.map((o) => o.name).join(",")}`)).toEqual([
            "a: 1 agents, agentmux-1",
            "b: 0 agents, agentmux-2",
        ]);
    });

    it("carries the review bot and CI, and has neither when none is given", () => {
        expect(buildAtlasGraph([machine("a")], [agent("x", "a")]).services).toEqual([]);
        const g = buildAtlasGraph([machine("a")], [agent("x", "a")], { services: [service("service:review"), service("service:checks")] });
        expect(g.services.map((s) => s.key)).toEqual(["service:review", "service:checks"]);
        expect(g.regions[0].orphans).toEqual([]);
    });
});
