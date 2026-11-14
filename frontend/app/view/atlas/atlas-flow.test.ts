// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { MessageFlowEvent } from "@/types/rpc/MessageFlowEvent";
import type { TaskFlowEvent } from "@/types/rpc/TaskFlowEvent";
import {
    flowCurve,
    flowCurves,
    flowEndResolver,
    flowsAt,
    LEAVING_MS,
    MAX_MESSAGES,
    MESSAGE_SHOWN_MS,
    messageFlow,
    TASK_SHOWN_MS,
    taskFlow,
    withFlow,
    type AtlasFlow,
} from "./atlas-flow";
import type { AtlasAgent, AtlasMachine } from "./atlas-graph";
import { NODE_H, NODE_W } from "./atlas-layout";

const message = (over: Partial<MessageFlowEvent> = {}): MessageFlowEvent => ({
    msg_id: "m1",
    at_ms: 1_000,
    from: { agent: "ann", host: "desk" },
    to: { agent: "bo", host: "desk" },
    via: "host",
    outcome: "delivered",
    ...over,
});

const task = (over: Partial<TaskFlowEvent> = {}): TaskFlowEvent => ({
    task: { number: 12, board: "main", host: "desk" },
    at_ms: 1_000,
    by: { agent: "ann", host: "desk" },
    to: { agent: "bo", host: "desk" },
    kind: "filed",
    ...over,
});

const line = (over: Partial<AtlasFlow> = {}): AtlasFlow => ({ ...messageFlow(message())!, ...over });

describe("messageFlow", () => {
    it("is a line from the sender to the receiver, in the tone of what became of it", () => {
        expect(messageFlow(message())).toMatchObject({ kind: "message", tone: "delivered", label: null, atMs: 1_000, leaving: false });
        expect(messageFlow(message({ outcome: "held" }))!.tone).toBe("waiting");
        expect(messageFlow(message({ outcome: "queued" }))!.tone).toBe("waiting");
        expect(messageFlow(message({ outcome: "refused" }))!.tone).toBe("refused");
    });

    it("is a new drawing for each thing that happens to one message, and nothing for a message to oneself", () => {
        const held = messageFlow(message({ outcome: "held" }))!;
        const delivered = messageFlow(message({ at_ms: 5_000 }))!;
        expect(held.id).not.toBe(delivered.id);
        expect(messageFlow(message({ to: { agent: "Ann", host: "desk" } }))).toBeNull();
    });
});

describe("taskFlow", () => {
    it("is a numbered line from who did it to who it is for", () => {
        expect(taskFlow(task())).toMatchObject({ kind: "task", tone: "task", label: "#12", from: { agent: "ann" }, to: { agent: "bo" } });
    });

    it("is nothing with one end only: the operator filed it, it is for anyone, or it went back to nobody", () => {
        expect(taskFlow(task({ by: undefined }))).toBeNull();
        expect(taskFlow(task({ to: undefined, kind: "handed_back" }))).toBeNull();
        expect(taskFlow(task({ to: { agent: "ann", host: "desk" } }))).toBeNull();
    });
});

describe("flowsAt", () => {
    it("keeps a line for its time, marks its last second, then drops it", () => {
        const flows = [line()];
        expect(flowsAt(flows, 1_000 + MESSAGE_SHOWN_MS - LEAVING_MS - 1)).toBe(flows);
        const leaving = flowsAt(flows, 1_000 + MESSAGE_SHOWN_MS - LEAVING_MS);
        expect(leaving).toEqual([{ ...flows[0], leaving: true }]);
        expect(flowsAt(leaving, 1_000 + MESSAGE_SHOWN_MS - 1)).toBe(leaving);
        expect(flowsAt(leaving, 1_000 + MESSAGE_SHOWN_MS)).toEqual([]);
    });

    it("keeps a task far longer than a message", () => {
        const flows = [line(), taskFlow(task())!];
        expect(flowsAt(flows, 1_000 + MESSAGE_SHOWN_MS).map((f) => f.kind)).toEqual(["task"]);
        expect(flowsAt(flows, 1_000 + TASK_SHOWN_MS)).toEqual([]);
    });
});

describe("withFlow", () => {
    it("adds a line, and not the one srv already reported", () => {
        const one = withFlow([], line(), 2_000);
        expect(one).toHaveLength(1);
        expect(withFlow(one, line(), 2_000)).toBe(one);
    });

    it("replaces the older drawing between the same two agents, and keeps the answer the other way", () => {
        const first = line();
        const again = line({ id: "again", atMs: 3_000 });
        const answer = line({ id: "answer", from: first.to, to: first.from, atMs: 3_500 });
        const flows = withFlow(withFlow(withFlow([], first, 2_000), again, 3_000), answer, 3_500);
        expect(flows.map((f) => f.id)).toEqual(["again", "answer"]);
        expect(withFlow(flows, line({ id: "late", atMs: 2_500 }), 3_600)).toBe(flows);
    });

    it("leaves out one whose time was already up, and drops the oldest past the most it shows", () => {
        expect(withFlow([], line(), 1_000 + MESSAGE_SHOWN_MS)).toEqual([]);
        let flows: readonly AtlasFlow[] = [];
        for (let i = 0; i <= MAX_MESSAGES; i++) {
            flows = withFlow(flows, line({ id: `m${i}`, to: { agent: `agent${i}`, host: "desk" } }), 2_000);
        }
        expect(flows).toHaveLength(MAX_MESSAGES);
        expect(flows[0].id).toBe("m1");
    });

    it("counts tasks and messages apart", () => {
        const flows = withFlow(withFlow([], line(), 2_000), taskFlow(task())!, 2_000);
        expect(flows.map((f) => f.kind)).toEqual(["message", "task"]);
    });
});

describe("flowEndResolver", () => {
    const machine = (key: string, title: string, unnamed = false): AtlasMachine => ({ key, title, badge: "", platform: null, stale: false, seenAtMs: null, unnamed });
    const agent = (name: string, m: string, over: Partial<AtlasAgent> = {}): AtlasAgent => ({
        key: `${m}/${name}`,
        name,
        provider: null,
        state: "idle",
        line: null,
        machine: m,
        agentId: null,
        seat: null,
        contextTokens: null,
        currentTool: null,
        blockId: null,
        runtimeId: null,
        remote: m !== "local",
        stale: false,
        ...over,
    });
    const machines = new Map([machine("local", "Desk"), machine("dev", "desk · dev"), machine("lab", "lab-1")].map((m) => [m.key, m] as const));

    it("finds an agent by the name messages reach it by, on the computer the end names", () => {
        const resolve = flowEndResolver([agent("Ann", "local"), agent("Pulp", "local", { route: "pulse" }), agent("Ann", "lab")], machines);
        expect(resolve({ agent: "ann", host: "desk" })).toBe("local/Ann");
        expect(resolve({ agent: "ANN", host: "lab-1" })).toBe("lab/Ann");
        expect(resolve({ agent: "pulse", host: "desk" })).toBe("local/Pulp");
        expect(resolve({ agent: "pulp", host: "desk" })).toBeNull();
    });

    it("takes this instance's own agent when two channels of one computer have the name", () => {
        const resolve = flowEndResolver([agent("Ann", "dev"), agent("Ann", "local")], machines);
        expect(resolve({ agent: "ann", host: "desk" })).toBe("local/Ann");
    });

    it("with no host, takes the only agent of that name, and none when it cannot tell two apart", () => {
        expect(flowEndResolver([agent("Cy", "lab")], machines)({ agent: "cy" })).toBe("lab/Cy");
        expect(flowEndResolver([agent("Cy", "lab"), agent("Cy", "dev")], machines)({ agent: "cy" })).toBeNull();
    });

    it("draws no line to a same-named agent on another computer than the one named", () => {
        const resolve = flowEndResolver([agent("Ann", "local")], machines);
        expect(resolve({ agent: "ann", host: "lab-1" })).toBeNull();
        expect(resolve({ agent: "ann", host: "elsewhere" })).toBeNull();
        expect(resolve({ agent: "github-consumer" })).toBeNull();
    });

    it("takes a host no machine is called as this computer, while its own name is not known", () => {
        const unnamed = new Map([machine("local", "This computer", true), machine("lab", "lab-1")].map((m) => [m.key, m] as const));
        const resolve = flowEndResolver([agent("Ann", "local"), agent("Ann", "lab")], unnamed);
        expect(resolve({ agent: "ann", host: "desk" })).toBe("local/Ann");
        expect(resolve({ agent: "ann", host: "lab-1" })).toBe("lab/Ann");
    });
});

describe("flowCurve", () => {
    it("runs from the edge of one node to the edge of the other", () => {
        const curve = flowCurve({ x: 0, y: 0 }, { x: 400, y: 0 })!;
        expect(curve.d.startsWith(`M${NODE_W},${NODE_H / 2} Q`)).toBe(true);
        expect(curve.end).toEqual({ x: 400, y: NODE_H / 2 });
    });

    it("bows a line and its answer to opposite sides", () => {
        const there = flowCurve({ x: 0, y: 0 }, { x: 400, y: 0 })!;
        const back = flowCurve({ x: 400, y: 0 }, { x: 0, y: 0 })!;
        expect(there.label.y).toBeLessThan(NODE_H / 2);
        expect(back.label.y).toBeGreaterThan(NODE_H / 2);
    });

    it("puts its label on a stretch no node covers", () => {
        const from = { x: 0, y: 0 };
        const to = { x: 600, y: 0 };
        const between = { x: 300, y: 0 };
        const middle = flowCurve(from, to)!.label;
        const moved = flowCurve(from, to, [from, between, to])!.label;
        expect(middle.x).toBe(370);
        // The node in the middle covers the line's middle: the label goes to the nearest stretch that is clear.
        expect(moved.x).toBeLessThan(between.x);
        expect(moved.x).toBeGreaterThan(from.x + NODE_W);
    });

    it("puts its label beside the node it lands on when a row of nodes covers the whole line", () => {
        // Three agents side by side, as the grid places them: the line from the third to the first runs behind the
        // second, and the gap between two of them is too narrow for a number.
        const row = [0, 154, 308].map((x) => ({ x, y: 0 }));
        const [first, , third] = row;
        // It bows down on its way left, so the number goes under the corner it lands on.
        expect(flowCurve(third, first, row)!.label).toEqual({ x: NODE_W, y: NODE_H + 7 });
        // The answer bows up, so its number goes over the corner it lands on.
        expect(flowCurve(first, third, row)!.label).toEqual({ x: 308, y: -7 });
        // A second row under the first leaves just the room a number needs.
        const grid = [...row, ...row.map((p) => ({ x: p.x, y: NODE_H + 14 }))];
        expect(flowCurve(third, first, grid)!.label).toEqual({ x: NODE_W, y: NODE_H + 7 });
    });

    it("takes the other side of the node it lands on when the first is covered", () => {
        const row = [0, 154, 308].map((x) => ({ x, y: 0 }));
        const under = { x: 70, y: NODE_H + 4 };
        expect(flowCurve(row[2], row[0], [...row, under])!.label).toEqual({ x: NODE_W, y: -7 });
    });

    it("keeps the middle for its label when every stretch and both sides of where it lands are covered", () => {
        const from = { x: 0, y: 0 };
        const to = { x: 600, y: 0 };
        const wall = [-30, 0, 30].flatMap((y) => [0, 100, 200, 300, 400, 500, 600].map((x) => ({ x, y })));
        expect(flowCurve(from, to, wall)!.label).toEqual(flowCurve(from, to)!.label);
    });

    it("is nothing for two nodes on top of each other, or touching", () => {
        expect(flowCurve({ x: 10, y: 10 }, { x: 10, y: 10 })).toBeNull();
        expect(flowCurve({ x: 0, y: 0 }, { x: NODE_W, y: 0 })).toBeNull();
    });
});

describe("flowCurves", () => {
    const row = [0, 154, 308].map((x) => ({ x, y: 0 }));
    const [first, second, third] = row;
    const labels = (lines: Parameters<typeof flowCurves>[0], nodes = row) =>
        [...flowCurves(lines, nodes).values()].map((curve) => curve.label);

    it("gives two lines that land on one corner of a node a label each", () => {
        // Two tasks handed to the first agent, by its neighbour and by the third: both lines land on its right
        // corner from below, so alone each would put its number under that corner.
        const [near, far] = labels([
            { id: "a", from: second, to: first, label: "#1" },
            { id: "b", from: third, to: first, label: "#3" },
        ]);
        expect(near).toEqual({ x: NODE_W, y: NODE_H + 7 });
        // The second takes the other side of the node.
        expect(far).toEqual({ x: NODE_W, y: -7 });
    });

    it("steps along the node's edge when both sides of the corner are taken", () => {
        const fourth = { x: 462, y: 0 };
        const [, , last] = labels(
            [
                { id: "a", from: second, to: first, label: "#47601" },
                { id: "b", from: third, to: first, label: "#47602" },
                { id: "c", from: fourth, to: first, label: "#47603" },
            ],
            [...row, fourth]
        );
        expect(last).toEqual({ x: NODE_W - 44, y: NODE_H + 7 });
    });

    it("puts the numbers of a line and its answer next to each other, in the gap the lines cross", () => {
        // One agent over another, as By seat places them: the two lines bow apart by less than a number's width.
        const over = { x: 0, y: 0 };
        const under = { x: 0, y: NODE_H + 40 };
        const alone = flowCurve(under, over, [over, under])!.label;
        const pair = (down: string, up: string) =>
            labels(
                [
                    { id: "a", from: over, to: under, label: down },
                    { id: "b", from: under, to: over, label: up },
                ],
                [over, under]
            );
        for (const [a, b, apart] of [
            ["#4", "#1", 16],
            ["#47601", "#47602", 42],
        ] as const) {
            const [down, up] = pair(a, b);
            expect(down).toEqual(flowCurve(over, under, [over, under])!.label);
            // The second stays at its own height, on the side of the first that its line is on.
            expect(up.y).toBe(alone.y);
            expect(Math.sign(up.x - down.x)).toBe(Math.sign(alone.x - down.x));
            expect(Math.abs(up.x - down.x)).toBeCloseTo(apart, 5);
        }
    });

    it("leaves a line and its answer their own numbers when the lines bow far enough apart", () => {
        const left = { x: 0, y: 0 };
        const right = { x: 600, y: 0 };
        const [there, back] = labels(
            [
                { id: "a", from: left, to: right, label: "#47601" },
                { id: "b", from: right, to: left, label: "#47602" },
            ],
            [left, right]
        );
        expect(there).toEqual(flowCurve(left, right, [left, right])!.label);
        expect(back).toEqual(flowCurve(right, left, [left, right])!.label);
    });

    it("places no label for a line without one, and leaves out a line with no curve", () => {
        const curves = flowCurves(
            [
                { id: "a", from: second, to: first, label: "#1" },
                { id: "m", from: third, to: first, label: null },
                { id: "x", from: first, to: first, label: "#2" },
                { id: "b", from: third, to: first, label: "#3" },
            ],
            row
        );
        expect([...curves.keys()]).toEqual(["a", "m", "b"]);
        // The message's line lands on the same corner and takes no label's place.
        expect(curves.get("b")!.label).toEqual({ x: NODE_W, y: -7 });
    });
});
