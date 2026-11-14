// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { KanbanTask, TaskState } from "../kanban/kanban-types";
import { agentWork, MAX_CARD_TASKS } from "./atlas-work";

function task(num: number, block: string | null, over: { state?: TaskState; pr?: string; branch?: string; repo?: string; at?: number } = {}): KanbanTask {
    return {
        title: `Task ${num}`,
        state: over.state ?? "claimed",
        updated_at: over.at ?? num,
        card: { num, repo: over.repo ?? "", branch: over.branch ?? "", pr_url: over.pr ?? "" },
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
        expect(work).toEqual({ repo: "agentmuxai/agentmux", branch: "ann/fix", tasks: [] });
    });

    it("falls back to the branch a held task was filed with, repository and branch together", () => {
        const facts = [{ block_id: "b1", repo: "o/elsewhere", branch: null }];
        const work = agentWork("b1", facts, [task(4, "b1", { repo: "o/r", branch: "ann/four" })]);
        expect(work?.repo).toBe("o/r");
        expect(work?.branch).toBe("ann/four");
    });

    it("keeps the repository alone when no branch is known anywhere", () => {
        expect(agentWork("b1", [{ block_id: "b1", repo: "o/r" }], [])).toEqual({ repo: "o/r", branch: null, tasks: [] });
    });

    it("lists the tasks the agent holds, newest first, with their pull requests", () => {
        const tasks = [
            task(7, "b1", { pr: "https://github.com/o/r/pull/70", at: 100 }),
            task(8, "b1", { at: 300 }),
            task(9, "b2", { at: 400 }),
            task(10, null, { at: 500 }),
        ];
        expect(agentWork("b1", [], tasks)?.tasks).toEqual([
            { num: 8, title: "Task 8", pr: null },
            { num: 7, title: "Task 7", pr: { url: "https://github.com/o/r/pull/70", label: "#70" } },
        ]);
    });

    it("leaves out a task that is over, and lists no more than the card has room for", () => {
        const over = (["done", "failed", "cancelled"] as const).map((state, i) => task(20 + i, "b1", { state }));
        const held = [1, 2, 3, 4, 5].map((n) => task(n, "b1", { at: n }));
        const got = agentWork("b1", [], [...over, ...held])!.tasks.map((t) => t.num);
        expect(got).toHaveLength(MAX_CARD_TASKS);
        expect(got).toEqual([5, 4, 3]);
    });

    it("counts a task in review or waiting on a question as held", () => {
        const got = agentWork("b1", [], [task(1, "b1", { state: "review" }), task(2, "b1", { state: "blocked" })])!.tasks;
        expect(got.map((t) => t.num).sort()).toEqual([1, 2]);
    });
});
