// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { KanbanTask } from "../kanban/kanban-types";
import type { AtlasAgent } from "./atlas-graph";
import { NODE_H, nodeHeight } from "./atlas-layout";
import { linesOf, nodeHeights, nodeTasksOf, showTaskLines, TASK_LINES_MIN_K } from "./atlas-node-tasks";
import { MAX_NODE_TASKS } from "./atlas-work";

function agent(name: string, over: Partial<AtlasAgent> = {}): AtlasAgent {
    return {
        key: `local/${name}`,
        name,
        provider: null,
        state: "working",
        line: null,
        machine: "local",
        agentId: `uid-${name}`,
        seat: null,
        contextTokens: null,
        currentTool: null,
        blockId: `b-${name}`,
        runtimeId: null,
        remote: false,
        stale: false,
        ...over,
    };
}

function held(num: number, block: string): KanbanTask {
    return {
        title: `Task ${num}`,
        state: "claimed",
        updated_at: num,
        priority: 0,
        target_agent: "",
        target_agent_uid: "",
        card: { num, pr_url: "" },
        holder: { name: "x", block_id: block, state: "working" },
    } as unknown as KanbanTask;
}

describe("what the nodes list", () => {
    it("lists the tasks of each of this instance's agents, by node key, and nothing for one with none", () => {
        const tasks = [held(1, "b-ann"), held(2, "b-ann"), held(3, "b-bob")];
        const got = nodeTasksOf([agent("ann"), agent("bob"), agent("cy")], tasks);
        expect([...got.keys()]).toEqual(["local/ann", "local/bob"]);
        expect(got.get("local/ann")!.shown.map((t) => t.num)).toEqual([2, 1]);
    });

    it("lists none for an agent of another instance: its tasks are on its machine's board", () => {
        const remote = agent("ann", { remote: true, blockId: null });
        expect(nodeTasksOf([remote], [held(1, "b-ann")]).size).toBe(0);
    });

    it("lists none when the board has not come", () => {
        expect(nodeTasksOf([agent("ann")], []).size).toBe(0);
    });

    it("makes a node tall for its lines, and one more for the line that counts the rest", () => {
        const few = nodeTasksOf([agent("ann")], [held(1, "b-ann"), held(2, "b-ann")]).get("local/ann")!;
        expect(linesOf(few)).toBe(2);
        const many = nodeTasksOf(
            [agent("ann")],
            Array.from({ length: MAX_NODE_TASKS + 2 }, (_, i) => held(i + 1, "b-ann"))
        ).get("local/ann")!;
        expect(linesOf(many)).toBe(MAX_NODE_TASKS + 1);
        expect(nodeHeights(new Map([["local/ann", many]])).get("local/ann")).toBe(nodeHeight(MAX_NODE_TASKS + 1));
        expect(nodeHeight(MAX_NODE_TASKS + 1)).toBeGreaterThan(NODE_H);
    });
});

describe("when the nodes list their tasks", () => {
    it("does when the fitted map with the lines is read at the least scale or more", () => {
        expect(showTaskLines(TASK_LINES_MIN_K, false, 0.3)).toBe(true);
        expect(showTaskLines(1.2, false, 0.3)).toBe(true);
    });

    it("does not in a busy map's fitted view, which says only how many", () => {
        expect(showTaskLines(0.6, false, 0.6)).toBe(false);
        expect(showTaskLines(0.6, false, 1.5)).toBe(false);
    });

    it("does once the person has zoomed in to the least scale, and stops when they zoom out", () => {
        expect(showTaskLines(0.6, true, TASK_LINES_MIN_K)).toBe(true);
        expect(showTaskLines(0.6, true, TASK_LINES_MIN_K - 0.01)).toBe(false);
    });
});
