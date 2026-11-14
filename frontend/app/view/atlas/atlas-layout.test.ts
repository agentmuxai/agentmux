// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { buildAtlasGraph, type AtlasAgent, type AtlasExtras, type AtlasMachine } from "./atlas-graph";
import { layoutAtlas, layoutBySeat, NODE_H, NODE_W, ROW_MAX_W, SERVICES_REGION } from "./atlas-layout";

const machine = (key: string): AtlasMachine => ({ key, title: key, badge: "LAN", platform: null, stale: false, seenAtMs: null });

function agent(name: string, m: string, lead?: string, state: AtlasAgent["state"] = "idle"): AtlasAgent {
    return {
        key: `${m}/${name}`,
        name,
        provider: null,
        state,
        line: null,
        machine: m,
        agentId: `id-${name}`,
        seat: lead ? { role: "worker", qualifier: null, lead: { id: `id-${lead}`, name: lead }, area: null } : null,
        contextTokens: null,
        currentTool: null,
        blockId: null,
        runtimeId: null,
        remote: false,
        stale: false,
    };
}

const overlap = (a: { x: number; y: number }, b: { x: number; y: number }) =>
    Math.abs(a.x - b.x) < NODE_W && Math.abs(a.y - b.y) < NODE_H;

describe("layoutAtlas", () => {
    it("lays out nothing for no agents, and one agent for one", () => {
        expect(layoutAtlas(buildAtlasGraph([machine("a")], []))).toMatchObject({ width: 0, height: 0 });
        const one = layoutAtlas(buildAtlasGraph([machine("a")], [agent("x", "a")]));
        expect(one.nodes.size).toBe(1);
        expect(one.regions).toHaveLength(1);
        expect(one.width).toBeGreaterThan(0);
    });

    it("puts a lead above its reports and centres it over them", () => {
        const l = layoutAtlas(
            buildAtlasGraph([machine("a")], [agent("boss", "a"), agent("w1", "a", "boss"), agent("w2", "a", "boss")])
        );
        const boss = l.nodes.get("a/boss")!;
        const w1 = l.nodes.get("a/w1")!;
        const w2 = l.nodes.get("a/w2")!;
        expect(w1.y).toBeGreaterThan(boss.y);
        expect(w1.y).toBe(w2.y);
        expect(boss.x).toBeCloseTo((w1.x + w2.x) / 2);
    });

    it("puts agents with no lead and no reports in a grid below the trees, and no two nodes overlap", () => {
        const agents = [agent("boss", "a"), agent("w", "a", "boss")];
        for (let i = 0; i < 12; i++) agents.push(agent(`solo${i}`, "a"));
        const l = layoutAtlas(buildAtlasGraph([machine("a")], agents));
        const tree = l.nodes.get("a/w")!;
        expect(l.nodes.get("a/solo0")!.y).toBeGreaterThan(tree.y);
        const all = [...l.nodes.values()];
        for (let i = 0; i < all.length; i++) for (let j = i + 1; j < all.length; j++) expect(overlap(all[i], all[j])).toBe(false);
    });

    it("gives the same layout for the same agents", () => {
        const agents = [agent("boss", "a"), agent("w1", "a", "boss"), agent("solo", "a")];
        const g = buildAtlasGraph([machine("a")], agents);
        expect(layoutAtlas(g)).toEqual(layoutAtlas(buildAtlasGraph([machine("a")], agents)));
    });

    it("wraps regions to a new row and keeps every node inside its region", () => {
        const machines = ["a", "b", "c", "d", "e"].map(machine);
        const agents = machines.flatMap((m) => ["p", "q", "r", "s", "t"].map((n) => agent(n, m.key)));
        const l = layoutAtlas(buildAtlasGraph(machines, agents));
        expect(l.width).toBeLessThanOrEqual(ROW_MAX_W);
        expect(new Set(l.regions.map((r) => r.y)).size).toBeGreaterThan(1);
        for (const region of l.regions) {
            const own = [...l.nodes].filter(([k]) => k.startsWith(`${region.key}/`));
            for (const [, p] of own) {
                expect(p.x).toBeGreaterThanOrEqual(region.x);
                expect(p.x + NODE_W).toBeLessThanOrEqual(region.x + region.w);
                expect(p.y).toBeGreaterThanOrEqual(region.y);
                expect(p.y + NODE_H).toBeLessThanOrEqual(region.y + region.h);
            }
        }
    });

    it("makes room for the folded chip, and a region of only stopped agents has the chip alone", () => {
        const l = layoutAtlas(
            buildAtlasGraph(
                [machine("a"), machine("b")],
                [agent("x", "a"), agent("y", "a", undefined, "stopped"), agent("z", "b", undefined, "stopped")]
            )
        );
        const [a, b] = l.regions;
        expect(a.foldY).toBeGreaterThan(l.nodes.get("a/x")!.y);
        expect(b.h).toBeLessThan(a.h);
        expect(l.nodes.size).toBe(1);
    });

    it("keeps agents in a lead cycle, not lost", () => {
        const a = agent("a1", "m", "a2");
        const b = agent("a2", "m", "a1");
        const l = layoutAtlas({
            regions: [{ ...machine("m"), shown: [a, b], folded: [], orphans: [] }],
            edges: [
                { from: "m/a2", to: "m/a1", kind: "lead" },
                { from: "m/a1", to: "m/a2", kind: "lead" },
            ],
            services: [],
        });
        expect(l.nodes.size).toBe(2);
    });
});

describe("what is not an agent", () => {
    const extras: AtlasExtras = {
        orphans: [{ key: "container:c1", machine: "a", name: "c1", state: "running", image: "i:1", sinceMs: 0 }],
        services: [
            { key: "service:review", name: "Review bot", icon: "robot", state: "ok", sub: "", lines: [] },
            { key: "service:checks", name: "CI", icon: "circle-check", state: "ok", sub: "", lines: [] },
        ],
    };
    const agents = [agent("boss", "a"), agent("w1", "a", "boss"), agent("x", "a"), agent("y", "b")];
    const graph = () => buildAtlasGraph([machine("a"), machine("b")], agents, extras);

    it("stands a container no agent uses with its machine's agents, and the review bot and CI in a region under the machines", () => {
        const l = layoutAtlas(graph());
        expect(l.regions.map((r) => r.key)).toEqual(["a", "b", SERVICES_REGION]);
        expect(l.nodes.size).toBe(agents.length + 3);
        const [a, , services] = l.regions;
        const inside = (key: string, box: { x: number; y: number; w: number; h: number }) => {
            const p = l.nodes.get(key)!;
            return p.x >= box.x && p.x + NODE_W <= box.x + box.w && p.y >= box.y && p.y + NODE_H <= box.y + box.h;
        };
        expect(inside("container:c1", a)).toBe(true);
        expect(inside("service:review", services)).toBe(true);
        expect(inside("service:checks", services)).toBe(true);
        // On a row of its own: a line from it to an agent does not run along the agent's row, behind its neighbours.
        expect(services.x).toBe(0);
        for (const box of l.regions.slice(0, -1)) expect(services.y).toBeGreaterThan(box.y + box.h);
        expect(l.height).toBe(services.y + services.h);
        const placed = [...l.nodes.values()];
        for (let i = 0; i < placed.length; i++) {
            for (let j = i + 1; j < placed.length; j++) expect(overlap(placed[i], placed[j])).toBe(false);
        }
    });

    it("gives the review bot and CI the first row when no agent is drawn", () => {
        const l = layoutBySeat(buildAtlasGraph([], [], { services: extras.services }));
        expect([...l.nodes.keys()]).toEqual(["service:review", "service:checks"]);
        expect(l.nodes.get("service:review")!.y).toBe(l.nodes.get("service:checks")!.y);
        expect(l.width).toBeGreaterThan(NODE_W * 2);
    });

    it("has no region for the review bot and CI when there are none", () => {
        const l = layoutAtlas(buildAtlasGraph([machine("a")], agents));
        expect(l.regions.map((r) => r.key)).toEqual(["a"]);
    });

    it("stands them in a row of their own under the agents in the By seat layout", () => {
        const l = layoutBySeat(graph());
        expect(l.regions).toEqual([]);
        expect(l.nodes.size).toBe(agents.length + 3);
        const review = l.nodes.get("service:review")!;
        expect(l.nodes.get("service:checks")!.y).toBe(review.y);
        for (const [key, p] of l.nodes) {
            if (!key.startsWith("service:")) expect(p.y + NODE_H).toBeLessThan(review.y);
        }
        expect(l.height).toBeGreaterThan(review.y + NODE_H);
        const placed = [...l.nodes.values()];
        for (let i = 0; i < placed.length; i++) {
            for (let j = i + 1; j < placed.length; j++) expect(overlap(placed[i], placed[j])).toBe(false);
        }
    });
});
