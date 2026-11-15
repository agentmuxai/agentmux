// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { buildAtlasGraph, type AtlasAgent, type AtlasMachine } from "./atlas-graph";
import { edgePath, wrappedRoute, type Pt } from "./atlas-edge-path";
import { GAP_X, GAP_Y, layoutAtlas, layoutBySeat, NODE_H, NODE_W, type PaneShape } from "./atlas-layout";

const machine = (key: string): AtlasMachine => ({ key, title: key, badge: "LAN", platform: null, stale: false, seenAtMs: null });
function agent(name: string, lead?: string): AtlasAgent {
    return {
        key: `m/${name}`,
        name,
        provider: null,
        state: "idle",
        line: null,
        machine: "m",
        agentId: `id-${name}`,
        seat: lead ? { role: "worker", qualifier: null, lead: { id: `id-${lead}`, name: lead }, area: null } : null,
        contextTokens: null,
        currentTool: null,
        blockId: null,
        runtimeId: null,
        remote: false,
    } as unknown as AtlasAgent;
}

describe("wrappedRoute", () => {
    const lead: Pt = { x: 400, y: 0 };

    it("leaves a line the layout did not say is wrapped as the curve it always was, however far down the report stands", () => {
        const under: Pt = { x: 300, y: NODE_H + GAP_Y };
        // A report on another machine, or of another lead, can stand rows below: that line is still the curve.
        const far: Pt = { x: 900, y: NODE_H + GAP_Y + 3 * (NODE_H + 24) };
        expect(edgePath(lead, under)).toMatch(/^M\S+ C/);
        expect(edgePath(lead, far)).toMatch(/^M\S+ C/);
        expect(edgePath(lead, far, true)).not.toMatch(/ C/);
    });

    it("runs a wrapped report's line along the gap above the first row, down the gap left of its column, into its left side", () => {
        const wrapped: Pt = { x: 300, y: NODE_H + GAP_Y + NODE_H + 24 };
        const route = wrappedRoute(lead, wrapped);
        expect(route[0]).toEqual({ x: lead.x + NODE_W / 2, y: NODE_H });
        expect(route.at(-1)).toEqual({ x: wrapped.x, y: wrapped.y + NODE_H / 2 });
        // The vertical run is in the gap left of the column, 7 px from the card.
        const rail = route.filter((p) => p.x === wrapped.x - GAP_X / 2);
        expect(rail.length).toBe(2);
        expect(edgePath(lead, wrapped, true)).toContain("Q");
    });

    it("has no sideways run when the gap left of the column is straight under the lead", () => {
        const wrapped: Pt = { x: lead.x + NODE_W / 2 + GAP_X / 2, y: NODE_H + GAP_Y + NODE_H + 24 };
        const route = wrappedRoute(lead, wrapped);
        // Down from the lead, then the short stub into the card: no run along the top of the first row.
        expect(route).toHaveLength(3);
        expect(route[0].x).toBe(route[1].x);
        expect(route[1].y).toBe(route[2].y);
    });
});

/** Every point along a polyline, every 2 px. */
function along(route: readonly Pt[]): Pt[] {
    const out: Pt[] = [];
    for (let i = 1; i < route.length; i++) {
        const a = route[i - 1];
        const b = route[i];
        const n = Math.max(1, Math.ceil(Math.hypot(b.x - a.x, b.y - a.y) / 2));
        for (let k = 0; k <= n; k++) out.push({ x: a.x + ((b.x - a.x) * k) / n, y: a.y + ((b.y - a.y) * k) / n });
    }
    return out;
}

describe("the lines of a wrapped fleet", () => {
    const wideLeaders = () => {
        const agents: AtlasAgent[] = [agent("agentx"), agent("agenty")];
        for (let i = 0; i < 5; i++) agents.push(agent(`x${i}`, "agentx"));
        for (let i = 0; i < 9; i++) agents.push(agent(`y${i}`, "agenty"));
        return buildAtlasGraph([machine("m")], agents);
    };
    const panes: PaneShape[] = [
        { w: 640, h: 1700 },
        { w: 640, h: 640 },
        { w: 1000, h: 1000 },
    ];

    it("cross no card, in either layout, in any of the panes", () => {
        const g = wideLeaders();
        for (const layout of [layoutAtlas, layoutBySeat]) {
            for (const pane of panes) {
                const l = layout(g, pane);
                let wrappedSeen = 0;
                for (const e of g.edges) {
                    const from = l.nodes.get(e.from);
                    const to = l.nodes.get(e.to);
                    if (!from || !to) continue;
                    if (l.wrapped.get(e.to) !== e.from) continue;
                    const route = wrappedRoute(from, to);
                    wrappedSeen++;
                    for (const p of along(route)) {
                        for (const [key, n] of l.nodes) {
                            const inside = p.x > n.x + 1 && p.x < n.x + NODE_W - 1 && p.y > n.y + 1 && p.y < n.y + NODE_H - 1;
                            expect(inside, `${e.from}→${e.to} crosses ${key}`).toBe(false);
                        }
                    }
                }
                // The narrow panes do wrap, so this tests something.
                if (pane.w === 640) expect(wrappedSeen).toBeGreaterThan(0);
            }
        }
    });
});

describe("which reports the layout says it wrapped", () => {
    const g = () => {
        const agents: AtlasAgent[] = [agent("agenty")];
        for (let i = 0; i < 9; i++) agents.push(agent(`y${i}`, "agenty"));
        return buildAtlasGraph([machine("m")], agents);
    };

    it("names only reports on a later row than the one under their lead, with their lead", () => {
        const l = layoutAtlas(g(), { w: 640, h: 1700 });
        expect(l.wrapped.size).toBeGreaterThan(0);
        const lead = l.nodes.get("m/agenty")!;
        for (const [report, leadKey] of l.wrapped) {
            expect(leadKey).toBe("m/agenty");
            expect(l.nodes.get(report)!.y).toBeGreaterThan(lead.y + NODE_H + GAP_Y + 1);
        }
    });

    it("is empty when every lead's reports stand in one row", () => {
        expect(layoutAtlas(g(), { w: 4000, h: 1500 }).wrapped.size).toBe(0);
        expect(layoutAtlas(g()).wrapped.size).toBe(0);
    });
});
