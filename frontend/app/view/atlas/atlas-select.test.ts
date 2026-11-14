// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { AtlasAgent } from "./atlas-graph";
import { NODE_H, NODE_W } from "./atlas-layout";
import { isDrag, nodesInRect, rectBetween, selectionKeyOf, toMapRect } from "./atlas-select";

const agent = (over: Partial<AtlasAgent>): AtlasAgent => ({
    key: "k",
    name: "Ann",
    provider: null,
    state: "idle",
    line: null,
    machine: "local",
    agentId: null,
    seat: null,
    contextTokens: null,
    currentTool: null,
    blockId: null,
    runtimeId: null,
    remote: false,
    stale: false,
    ...over,
});

describe("rectBetween", () => {
    it("is the same box whichever way it was dragged", () => {
        expect(rectBetween(10, 20, 50, 80)).toEqual({ x: 10, y: 20, w: 40, h: 60 });
        expect(rectBetween(50, 80, 10, 20)).toEqual({ x: 10, y: 20, w: 40, h: 60 });
    });

    it("takes a few pixels of travel to be a drag", () => {
        expect(isDrag(rectBetween(10, 10, 12, 11))).toBe(false);
        expect(isDrag(rectBetween(10, 10, 10, 40))).toBe(true);
    });
});

describe("toMapRect", () => {
    it("undoes the pan and the zoom", () => {
        expect(toMapRect({ x: 116, y: 66, w: 100, h: 50 }, { x: 16, y: 16, k: 0.5 })).toEqual({ x: 200, y: 100, w: 200, h: 100 });
    });
});

describe("nodesInRect", () => {
    const nodes = new Map([
        ["a", { x: 0, y: 0 }],
        ["b", { x: 200, y: 0 }],
        ["c", { x: 0, y: 200 }],
    ]);

    it("takes every node the box touches, a corner included, and no other", () => {
        expect(nodesInRect(nodes, { x: NODE_W - 2, y: NODE_H - 2, w: 80, h: 20 })).toEqual(["a", "b"]);
        expect(nodesInRect(nodes, { x: -50, y: -50, w: 1000, h: 1000 })).toEqual(["a", "b", "c"]);
        expect(nodesInRect(nodes, { x: NODE_W + 5, y: NODE_H + 5, w: 20, h: 20 })).toEqual([]);
    });
});

describe("selectionKeyOf", () => {
    it("is the pane's block id for this instance's agent, and nothing for one with no pane", () => {
        expect(selectionKeyOf(agent({ key: "b1", blockId: "b1" }))).toBe("b1");
        expect(selectionKeyOf(agent({ key: "name:Ann" }))).toBeNull();
    });

    it("is the row key for another instance's agent, unless its machine has gone stale", () => {
        expect(selectionKeyOf(agent({ key: "remote:lab/Cy", remote: true }))).toBe("remote:lab/Cy");
        expect(selectionKeyOf(agent({ key: "remote:lab/Cy", remote: true, stale: true }))).toBeNull();
    });

    it("is nothing for a stopped agent, which is not a node", () => {
        expect(selectionKeyOf(agent({ key: "remote:lab/Cy", remote: true, state: "stopped" }))).toBeNull();
    });
});
