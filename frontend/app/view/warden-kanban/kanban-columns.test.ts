// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import {
    backlogGroups,
    checklistProgress,
    columnOf,
    columnTasks,
    matchesQuery,
    needsReason,
    prLabel,
    rankBetween,
    requestedBy,
    reviewLoad,
    untrackedAgents,
} from "./kanban-columns";
import type { KanbanAgent, KanbanAgentState, KanbanTask, TaskState } from "./kanban-types";

function agent(name: string, state: KanbanAgentState, uid = `uid-${name}`): KanbanAgent {
    return { name, uid, block_id: `b-${name}`, state };
}

function task(num: number, state: TaskState, over: Partial<KanbanTask> = {}): KanbanTask {
    return {
        id: `w${num}`,
        title: `task ${num}`,
        payload: "",
        kind: "",
        target_agent: "",
        target_agent_uid: "",
        priority: 0,
        state,
        claimed_by: "",
        claimed_by_uid: "",
        attempts: 0,
        max_attempts: 3,
        created_by: "operator",
        created_at: num * 1000,
        updated_at: num * 1000,
        result: "",
        holder: null,
        ...over,
        card: {
            num,
            repo: "",
            branch: "",
            pr_url: "",
            review_required: false,
            checklist: [],
            question: "",
            note: "",
            requested_by: "",
            requested_by_kind: "",
            request_text: "",
            filed_by: "",
            updated_at: 0,
            ...(over.card ?? {}),
        },
    };
}

describe("which column a task is in", () => {
    it("puts waiting tasks in the Backlog, handed-in ones in Review, finished ones in Done", () => {
        expect(columnOf(task(1, "open"))).toBe("backlog");
        expect(columnOf(task(2, "held"))).toBe("backlog");
        expect(columnOf(task(3, "review"))).toBe("review");
        expect(columnOf(task(4, "done"))).toBe("done");
        expect(columnOf(task(5, "cancelled"))).toBe("done");
    });

    it("keeps a claimed task In progress while its holder works or idles", () => {
        expect(columnOf(task(1, "claimed", { holder: agent("Loap-3", "working") }))).toBe("progress");
        expect(columnOf(task(2, "claimed", { holder: agent("Loap-3", "idle") }))).toBe("progress");
    });

    it("moves it to Needs you when its holder asks, waits on a question, errors or is gone, and says why", () => {
        const asked = task(1, "blocked", { card: { question: "which branch?" } as KanbanTask["card"] });
        expect([columnOf(asked), needsReason(asked)]).toEqual(["needs", "asks: which branch?"]);
        const waiting = task(2, "claimed", { holder: agent("Loap-3", "waiting") });
        expect([columnOf(waiting), needsReason(waiting)]).toEqual(["needs", "question waiting in its pane"]);
        expect(needsReason(task(3, "claimed", { holder: agent("Loap-3", "error") }))).toBe("agent errored");
        expect(needsReason(task(4, "claimed", { holder: null }))).toBe("agent closed");
        expect(needsReason(task(5, "failed", { attempts: 3 }))).toBe("failed after 3 attempts");
        expect(columnOf(task(5, "failed"))).toBe("needs");
    });
});

describe("the Backlog", () => {
    it("groups tasks by the agent they wait for, Anyone last, each in srv's claim order", () => {
        const tasks = [
            task(1, "open", { target_agent: "Naki", created_at: 3000 }),
            task(2, "open"),
            task(3, "open", { target_agent: "Loap-3", created_at: 2000 }),
            task(4, "held", { target_agent: "Loap-3", created_at: 1000 }),
            task(5, "open", { target_agent: "Loap-3", created_at: 500, card: { rank: 2500 } as KanbanTask["card"] }),
            task(6, "claimed", { target_agent: "Loap-3" }),
        ];
        const groups = backlogGroups(tasks, [agent("Loap-3", "working")]);
        expect(groups.map((g) => g.label)).toEqual(["Loap-3", "Naki", "Anyone"]);
        expect(groups[0].tasks.map((t) => t.card.num)).toEqual([4, 3, 5]);
        expect(groups[0].agent?.state).toBe("working");
        expect(groups[1].agent).toBeUndefined();
    });

    it("puts higher priority first, as claim does", () => {
        const tasks = [task(1, "open"), task(2, "open", { priority: 5, created_at: 99_000 })];
        expect(columnTasks(tasks, "backlog").map((t) => t.card.num)).toEqual([2, 1]);
    });

    it("finds a rank between two tasks, or before or after one", () => {
        expect(rankBetween(1000, 2000)).toBe(1500);
        expect(rankBetween(1000, undefined)).toBe(2000);
        expect(rankBetween(undefined, 1000)).toBe(0);
    });
});

describe("untracked work", () => {
    it("is every agent at work that holds no task", () => {
        const agents = [agent("A", "working"), agent("B", "working"), agent("C", "idle"), agent("D", "waiting")];
        const tasks = [task(1, "claimed", { claimed_by: "B", claimed_by_uid: "uid-B" })];
        expect(untrackedAgents(agents, tasks).map((a) => a.name)).toEqual(["A", "D"]);
    });
});

describe("what a card says", () => {
    it("names who asked", () => {
        expect(
            requestedBy(
                task(1, "open", { card: { requested_by_kind: "operator", filed_by: "Loap-3" } as KanbanTask["card"] })
            )
        ).toBe("you, via Loap-3");
        expect(requestedBy(task(2, "open", { card: { requested_by_kind: "operator" } as KanbanTask["card"] }))).toBe(
            "you"
        );
        expect(
            requestedBy(
                task(3, "open", { card: { requested_by_kind: "agent", requested_by: "Naki" } as KanbanTask["card"] })
            )
        ).toBe("Naki");
    });

    it("counts the checklist and Review's load", () => {
        const done = { text: "a", status: "completed" };
        const todo = { text: "b", status: "pending" };
        expect(checklistProgress(task(1, "claimed", { card: { checklist: [done, todo] } as KanbanTask["card"] }))).toBe(
            "1/2"
        );
        expect(checklistProgress(task(2, "claimed"))).toBeNull();
        const r = reviewLoad(
            [task(1, "review", { updated_at: 1000 }), task(2, "review", { updated_at: 4000 }), task(3, "open")],
            10_000
        );
        expect(r).toEqual({ count: 2, oldestMs: 9000 });
    });

    it("matches the search box on title, number, agent and repository", () => {
        const t = task(12, "open", {
            title: "Write the spec",
            target_agent: "Loap-3",
            card: { repo: "agentmuxai/agentmux" } as KanbanTask["card"],
        });
        expect(["#12", "12", "spec", "loap", "agentmux", ""].every((q) => matchesQuery(t, q))).toBe(true);
        expect(matchesQuery(t, "naki")).toBe(false);
    });

    it("labels a PR link with its number", () => {
        expect(prLabel("https://github.com/agentmuxai/agentmux/pull/5248")).toBe("#5248");
        expect(prLabel("https://example.com/merge_requests/3")).toBe("PR");
    });
});
