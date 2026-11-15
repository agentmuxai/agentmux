// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { buildAtlasGraph, type AtlasAgent, type AtlasExtras, type AtlasMachine } from "./atlas-graph";
import {
    FIT_MAX_K,
    fitScale,
    layoutAtlas,
    layoutBySeat,
    MIN_READABLE_K,
    NODE_H,
    NODE_W,
    ROW_MAX_W,
    SERVICES_REGION,
    type PaneShape,
} from "./atlas-layout";

const machine = (key: string): AtlasMachine => ({
    key,
    title: key,
    badge: "LAN",
    platform: null,
    stale: false,
    seenAtMs: null,
});

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
        for (let i = 0; i < all.length; i++)
            for (let j = i + 1; j < all.length; j++) expect(overlap(all[i], all[j])).toBe(false);
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

/** A fleet like the operator's: five machines and about 35 agents, a lead with reports on the biggest. */
function fleet(extra: { machine?: string; name?: string } = {}) {
    const machines = ["narko", "area54", "starpower", "charlie", "laptop"].map(machine);
    const agents: AtlasAgent[] = [agent("agentx", "narko"), agent("agenty", "narko")];
    for (let i = 0; i < 4; i++) agents.push(agent(`w${i}`, "narko", i % 2 ? "agentx" : "agenty"));
    for (let i = 0; i < 6; i++) agents.push(agent(`n${i}`, "narko"));
    const counts: Record<string, number> = { area54: 6, starpower: 5, charlie: 5, laptop: 6 };
    for (const [m, n] of Object.entries(counts)) for (let i = 0; i < n; i++) agents.push(agent(`${m}${i}`, m));
    if (extra.name) agents.push(agent(extra.name, extra.machine ?? "laptop"));
    return buildAtlasGraph(machines, agents);
}

/** The scale the fitted view shows a layout at, and the share of the pane its map then covers. */
function shown(l: { width: number; height: number }, pane: PaneShape) {
    const k = Math.max(MIN_READABLE_K, Math.min(FIT_MAX_K, fitScale(l, pane)));
    return { k, cover: (Math.min(l.width * k, pane.w) * Math.min(l.height * k, pane.h)) / (pane.w * pane.h) };
}

describe("layoutAtlas for a pane", () => {
    const panes: Record<string, PaneShape> = {
        tall: { w: 640, h: 1700 },
        square: { w: 1000, h: 1000 },
        wide: { w: 1600, h: 700 },
    };

    it("lays out an empty fleet and one with only the services, in a pane, without throwing", () => {
        const pane = panes.tall;
        expect(layoutAtlas(buildAtlasGraph([], []), pane)).toMatchObject({ width: 0, height: 0 });
        const services: AtlasExtras = {
            orphans: [],
            services: [{ key: "service:ci", name: "CI", icon: "circle-check", state: "ok", sub: "", lines: [] }],
        };
        const l = layoutAtlas(buildAtlasGraph([], [], services), pane);
        expect(l.regions.map((r) => r.key)).toEqual([SERVICES_REGION]);
        expect(l.nodes.has("service:ci")).toBe(true);
    });

    it("fills a tall, a square and a wide pane with cards a person can read", () => {
        const g = fleet();
        for (const [name, pane] of Object.entries(panes)) {
            const l = layoutAtlas(g, pane);
            const s = shown(l, pane);
            expect(s.k, `${name}: scale`).toBeGreaterThanOrEqual(MIN_READABLE_K);
            expect(s.cover, `${name}: share of the pane`).toBeGreaterThan(0.55);
        }
    });

    it("covers far more of a tall pane than the fixed-width layout did", () => {
        const g = fleet();
        const before = shown(layoutAtlas(g), panes.tall).cover;
        const after = shown(layoutAtlas(g, panes.tall), panes.tall).cover;
        expect(after).toBeGreaterThan(before * 1.5);
    });

    it("forms a taller picture for a tall pane than for a wide one, from the same agents", () => {
        const g = fleet();
        const tall = layoutAtlas(g, panes.tall);
        const wide = layoutAtlas(g, panes.wide);
        expect(tall.height / tall.width).toBeGreaterThan(wide.height / wide.width);
    });

    it("keeps the machines in their order, with no two nodes overlapping, in every pane", () => {
        const g = fleet();
        for (const pane of Object.values(panes)) {
            const l = layoutAtlas(g, pane);
            expect(l.regions.map((r) => r.key)).toEqual(g.regions.map((r) => r.key));
            const all = [...l.nodes.values()];
            for (let i = 0; i < all.length; i++)
                for (let j = i + 1; j < all.length; j++) expect(overlap(all[i], all[j])).toBe(false);
        }
    });

    it("moves no machine when an agent joins a box that has a free grid slot, or when an agent's state changes", () => {
        const base = fleet({ name: "extra0" });
        const more = fleet({ name: "extra0" });
        // The same fleet again, one agent busy instead of idle, is the same picture.
        const busy = buildAtlasGraph(
            ["narko", "area54", "starpower", "charlie", "laptop"].map(machine),
            [...base.regions.flatMap((r) => r.shown)].map((a, i) => (i === 3 ? { ...a, state: "working" as const } : a))
        );
        for (const pane of Object.values(panes)) {
            const l = layoutAtlas(base, pane);
            expect(layoutAtlas(more, pane).regions).toEqual(l.regions);
            expect(layoutAtlas(busy, pane).regions).toEqual(l.regions);
        }
    });

    it("keeps the machines' order when an agent joins and a box grows", () => {
        const g = fleet();
        const grown = fleet({ machine: "narko", name: "newcomer" });
        for (const pane of Object.values(panes)) {
            expect(layoutAtlas(grown, pane).regions.map((r) => r.key)).toEqual(
                layoutAtlas(g, pane).regions.map((r) => r.key)
            );
        }
    });

    it("is the fixed layout when it has no pane to form for", () => {
        const g = fleet();
        expect(layoutAtlas(g, { w: 0, h: 0 }).regions).toEqual(layoutAtlas(g).regions);
    });
});

describe("layoutBySeat for a pane", () => {
    it("shapes the grid of agents with no lead to a tall and to a wide pane", () => {
        const g = fleet();
        const tall = layoutBySeat(g, { w: 640, h: 1700 });
        const wide = layoutBySeat(g, { w: 1600, h: 700 });
        expect(tall.height / tall.width).toBeGreaterThan(wide.height / wide.width);
        expect(shown(tall, { w: 640, h: 1700 }).k).toBeGreaterThanOrEqual(MIN_READABLE_K);
    });
});


/**
 * A fleet like the one a Linux look check found clipped: one machine with two leaders that have 5 and 6 reports (its box is
 * about 1,400 px wide with every report in one row), and four smaller machines.
 */
function wideLeaderFleet(midLeads = false) {
    const machines = ["narko", "area54", "gamerlove", "starpower", "charlie"].map(machine);
    const agents: AtlasAgent[] = [agent("agentx", "narko"), agent("agenty", "narko")];
    for (let i = 0; i < 5; i++) agents.push(agent(`x${i}`, "narko", "agentx"));
    if (midLeads) {
        agents.push(agent("m1", "narko", "agenty"), agent("m2", "narko", "agenty"));
        for (let i = 0; i < 3; i++) agents.push(agent(`ya${i}`, "narko", "m1"));
        for (let i = 0; i < 4; i++) agents.push(agent(`yb${i}`, "narko", "m2"));
    } else {
        for (let i = 0; i < 6; i++) agents.push(agent(`y${i}`, "narko", "agenty"));
    }
    const counts: Record<string, number> = { area54: 10, gamerlove: 6, starpower: 4, charlie: 1 };
    for (const [m, n] of Object.entries(counts)) for (let i = 0; i < n; i++) agents.push(agent(`${m}${i}`, m));
    return buildAtlasGraph(machines, agents);
}

describe("layoutAtlas wraps a lead's reports for a pane", () => {
    const panes: Record<string, PaneShape> = {
        tall: { w: 640, h: 1700 },
        square: { w: 1000, h: 1000 },
        wide: { w: 1600, h: 700 },
    };

    it("fits a machine with wide leaders in a tall and a square pane at a readable card size", () => {
        const g = wideLeaderFleet();
        // Unwrapped, the machine's box alone is wider than either pane at the readable floor.
        expect(fitScale(layoutAtlas(g), panes.tall)).toBeLessThan(MIN_READABLE_K);
        for (const pane of [panes.tall, panes.square]) {
            const l = layoutAtlas(g, pane);
            expect(fitScale(l, pane)).toBeGreaterThanOrEqual(MIN_READABLE_K);
        }
    });

    it("does not stand a node on another, or outside its machine's box", () => {
        for (const g of [wideLeaderFleet(), wideLeaderFleet(true)]) {
            for (const pane of Object.values(panes)) {
                const l = layoutAtlas(g, pane);
                const placed = [...l.nodes.values()];
                for (let i = 0; i < placed.length; i++) {
                    for (let j = i + 1; j < placed.length; j++) expect(overlap(placed[i], placed[j])).toBe(false);
                }
                for (const region of l.regions) {
                    for (const [k, p] of l.nodes) {
                        if (!k.startsWith(`${region.key}/`)) continue;
                        expect(p.x + NODE_W).toBeLessThanOrEqual(region.x + region.w);
                        expect(p.y + NODE_H).toBeLessThanOrEqual(region.y + region.h);
                    }
                }
            }
        }
    });

    it("wraps a lead's reports under it, centred, and keeps a lead above every report", () => {
        const g = wideLeaderFleet();
        const l = layoutAtlas(g, panes.tall);
        const lead = l.nodes.get("narko/agenty")!;
        const reports = [0, 1, 2, 3, 4, 5].map((i) => l.nodes.get(`narko/y${i}`)!);
        expect(new Set(reports.map((r) => r.y)).size).toBeGreaterThan(1);
        for (const r of reports) expect(r.y).toBeGreaterThan(lead.y);
        const first = reports.filter((r) => r.y === Math.min(...reports.map((q) => q.y)));
        const left = Math.min(...first.map((r) => r.x));
        const right = Math.max(...first.map((r) => r.x)) + NODE_W;
        expect(lead.x + NODE_W / 2).toBeCloseTo((left + right) / 2, 5);
    });

    it("leaves a lead's reports in one row when the pane is wide enough for it", () => {
        const g = wideLeaderFleet();
        const l = layoutAtlas(g, { w: 3000, h: 1200 });
        const rows = new Set([0, 1, 2, 3, 4, 5].map((i) => l.nodes.get(`narko/y${i}`)!.y));
        expect(rows.size).toBe(1);
    });

    it("never wraps a lead whose reports lead others, only the reports with none", () => {
        const l = layoutAtlas(wideLeaderFleet(true), panes.tall);
        const m1 = l.nodes.get("narko/m1")!;
        const m2 = l.nodes.get("narko/m2")!;
        expect(m1.y).toBe(m2.y);
    });

    it("fits the By seat layout the same way", () => {
        const g = wideLeaderFleet();
        const l = layoutBySeat(g, panes.tall);
        expect(fitScale(l, panes.tall)).toBeGreaterThanOrEqual(MIN_READABLE_K);
    });
});
