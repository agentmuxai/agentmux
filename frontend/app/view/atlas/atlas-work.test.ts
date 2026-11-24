// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { KanbanTask, TaskState } from "../kanban/kanban-types";
import { agentWork, MAX_CARD_TASKS, MAX_NODE_TASKS, nodeTasks, tasksOf } from "./atlas-work";

function task(
    num: number,
    block: string | null,
    over: { state?: TaskState; pr?: string; branch?: string; repo?: string; at?: number; for?: string; uid?: string; priority?: number } = {}
): KanbanTask {
    return {
        title: `Task ${num}`,
        state: over.state ?? "claimed",
        updated_at: over.at ?? num,
        priority: over.priority ?? 0,
        target_agent: over.for ?? "",
        target_agent_uid: over.uid ?? "",
        card: { num, repo: over.repo ?? "", branch: over.branch ?? "", pr_url: over.pr ?? "", rank: num },
        holder: block ? { name: "Ann", block_id: block, state: "working" } : null,
    } as unknown as KanbanTask;
}

describe("agentWork", () => {
    it("is nothing for an agent srv knows no work of", () => {
        expect(agentWork("b1", [], [])).toBeNull();
        expect(agentWork("b1", [{ block_id: "b2", repo: "o/r", branch: "main" }], [task(1, "b2")])).toBeNull();
    });

    it("reads the branch of the agent's own clone", () => {
        const work = agentWork("b1", [{ block_id: "b1", repo: "agentmuxai/agentmux", branch: "ann/fix" }], []);
        expect(work).toEqual({ repo: "agentmuxai/agentmux", branch: "ann/fix", tasks: [], more: 0 });
    });

    it("falls back to the branch a held task was filed with, repository and branch together", () => {
        const facts = [{ block_id: "b1", repo: "o/elsewhere", branch: null }];
        const work = agentWork("b1", facts, [task(4, "b1", { repo: "o/r", branch: "ann/four" })]);
        expect(work?.repo).toBe("o/r");
        expect(work?.branch).toBe("ann/four");
    });

    it("keeps the repository alone when no branch is known anywhere", () => {
        expect(agentWork("b1", [{ block_id: "b1", repo: "o/r" }], [])).toEqual({ repo: "o/r", branch: null, tasks: [], more: 0 });
    });

    it("lists the tasks the agent holds, newest first, with their pull requests", () => {
        const tasks = [
            task(7, "b1", { pr: "https://github.com/o/r/pull/70", at: 100 }),
            task(8, "b1", { at: 300 }),
            task(9, "b2", { at: 400 }),
            task(10, null, { at: 500 }),
        ];
        expect(agentWork("b1", [], tasks)?.tasks).toEqual([
            { num: 8, title: "Task 8", pr: null, column: "progress", held: true },
            { num: 7, title: "Task 7", pr: { url: "https://github.com/o/r/pull/70", label: "#70" }, column: "review", held: true },
        ]);
    });

    it("leaves out a task that is over, and lists no more than the card has room for", () => {
        const over = (["done", "failed", "cancelled"] as const).map((state, i) => task(20 + i, "b1", { state }));
        const held = Array.from({ length: MAX_CARD_TASKS + 2 }, (_, i) => task(i + 1, "b1", { at: i + 1 }));
        const work = agentWork("b1", [], [...over, ...held])!;
        expect(work.tasks).toHaveLength(MAX_CARD_TASKS);
        expect(work.tasks[0].num).toBe(MAX_CARD_TASKS + 2);
        expect(work.more).toBe(2);
    });

    it("counts a task in review or waiting on a question as held", () => {
        const got = agentWork("b1", [], [task(1, "b1", { state: "review" }), task(2, "b1", { state: "blocked" })])!.tasks;
        expect(got.map((t) => t.num).sort()).toEqual([1, 2]);
    });
});

describe("the tasks of an agent, as its node lists them", () => {
    const ann = { blockId: "b1", agentId: "uid-ann", route: null, name: "Ann" };

    it("lists those it holds, newest first, and then those waiting for it in the order srv hands them out", () => {
        const tasks = [
            task(1, "b1", { at: 10 }),
            task(2, "b1", { at: 30, state: "review", pr: "https://github.com/o/r/pull/9" }),
            task(3, null, { state: "open", for: "ann", priority: 0 }),
            task(4, null, { state: "held", for: "ann", priority: 5 }),
            task(5, null, { state: "open", for: "bob" }),
        ];
        const got = tasksOf(ann, tasks);
        expect(got.map((t) => [t.num, t.held])).toEqual([
            [2, true],
            [1, true],
            [4, false],
            [3, false],
        ]);
        expect(got[0].pr).toEqual({ url: "https://github.com/o/r/pull/9", label: "#9" });
    });

    it("names a waiting task's agent by its uid, or by its id or name in any case", () => {
        const byUid = task(1, null, { state: "open", uid: "uid-ann", for: "somebody-else" });
        const bySlug = task(2, null, { state: "open", for: "ANN" });
        const byRoute = task(3, null, { state: "open", for: "ann-2" });
        expect(tasksOf(ann, [byUid, bySlug, byRoute]).map((t) => t.num).sort()).toEqual([1, 2]);
        expect(tasksOf({ ...ann, route: "ann-2" }, [byRoute]).map((t) => t.num)).toEqual([3]);
    });

    it("leaves out a task that is over, one held by another agent and one nobody named", () => {
        const tasks = [
            task(1, "b1", { state: "done" }),
            task(2, "b2"),
            task(3, null, { state: "open", for: "" }),
            task(4, null, { state: "cancelled", for: "ann" }),
            task(5, "b2", { state: "open", for: "ann" }),
        ];
        expect(tasksOf(ann, tasks)).toEqual([]);
    });

    it("an agent with no pane holds nothing, but may have tasks waiting", () => {
        const tasks = [task(1, "b1"), task(2, null, { state: "open", for: "ann" })];
        expect(tasksOf({ ...ann, blockId: null }, tasks).map((t) => t.num)).toEqual([2]);
    });

    it("lists the first few on the node and counts the rest", () => {
        const many = Array.from({ length: MAX_NODE_TASKS + 3 }, (_, i) => task(i + 1, "b1", { at: i + 1 }));
        const node = nodeTasks(tasksOf(ann, many));
        expect(node.shown).toHaveLength(MAX_NODE_TASKS);
        expect(node.more).toBe(3);
        expect(node.total).toBe(MAX_NODE_TASKS + 3);
        expect(nodeTasks([])).toEqual({ shown: [], more: 0, total: 0 });
    });

    it("gives a held task the colour of the column the board puts it in", () => {
        const got = tasksOf(ann, [task(1, "b1", { state: "blocked" }), task(2, "b1", { state: "claimed", at: 0 })]);
        expect(got.map((t) => t.column).sort()).toEqual(["needs", "progress"]);
    });
});
