// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import {
    backlogGroups,
    checklistProgress,
    columnOf,
    columnStats,
    columnTasks,
    isSpecd,
    matchesQuery,
    needsCountOf,
    needsReason,
    quotaReason,
    parseAfter,
    parseCollapsed,
    prLabel,
    rankBetween,
    repoLabel,
    repoLanes,
    requestedBy,
    reviewLimitOf,
    reviewLoad,
    stageWord,
    takesDrops,
    toggleCollapsed,
    untrackedAgents,
    waitingOn,
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
    it("puts waiting tasks in the Backlog, handed-in ones in Review, finished ones in Merged", () => {
        expect(columnOf(task(1, "open"))).toBe("backlog");
        expect(columnOf(task(2, "held"))).toBe("backlog");
        expect(columnOf(task(3, "review"))).toBe("review");
        expect(columnOf(task(4, "done"))).toBe("merged");
        expect(columnOf(task(5, "cancelled"))).toBe("merged");
    });

    it("puts a spec or research task being worked In Research, any other In progress", () => {
        const working = { holder: agent("Naki-2", "working") };
        expect(columnOf(task(1, "claimed", { ...working, kind: "spec" }))).toBe("research");
        expect(columnOf(task(2, "claimed", { ...working, kind: "Research" }))).toBe("research");
        expect(columnOf(task(3, "claimed", { ...working, kind: "build" }))).toBe("progress");
        expect(columnOf(task(4, "claimed", working))).toBe("progress");
        // Needs you overrides the stage.
        expect(columnOf(task(5, "claimed", { kind: "spec", holder: agent("Naki-2", "error") }))).toBe("needs");
    });

    it("puts a waiting task whose spec was accepted in Spec'd, and only then", () => {
        const after = (state: TaskState, kind = "spec") => ({ id: "s", num: 9, title: "Spec: x", state, kind });
        const build = (...a: ReturnType<typeof after>[]) => task(1, "open", { kind: "build", card: { after: a } as KanbanTask["card"] });
        for (const s of ["open", "claimed", "blocked", "review", "failed"] as TaskState[]) {
            expect(columnOf(build(after(s)))).toBe("backlog");
        }
        expect(columnOf(build(after("done")))).toBe("specd");
        expect(isSpecd(build(after("done", "Spec")))).toBe(true);
        expect(columnOf(build(after("cancelled"))), "a cancelled spec approved nothing").toBe("backlog");
        expect(columnOf(build(after("done", "build"))), "a plain predecessor").toBe("backlog");
        expect(columnOf(build(after("done"), { ...after("open", ""), id: "p" })), "still waiting on another").toBe(
            "backlog"
        );
        expect(columnOf(build(after("done"), { ...after("done", ""), id: "p" }))).toBe("specd");
        const held = build(after("done"));
        held.state = "held";
        expect(columnOf(held), "held, and still approved").toBe("specd");
        // An older srv sends no kind: Backlog, as before.
        expect(columnOf(build({ id: "s", num: 9, title: "x", state: "done" } as ReturnType<typeof after>))).toBe("backlog");
    });

    it("puts a held task with a PR in Review, then Merged when it merges, then Released", () => {
        const working = { holder: agent("Naki-2", "working") };
        const pr = (over: Partial<KanbanTask["card"]>) => ({ card: { pr_url: "https://github.com/o/r/pull/1", ...over } as KanbanTask["card"] });
        expect(columnOf(task(1, "claimed", { ...working, ...pr({}) }))).toBe("review");
        expect(columnOf(task(2, "claimed", { ...working, ...pr({ pr: { merged_at: 5 } }) }))).toBe("merged");
        expect(columnOf(task(3, "done", pr({ pr: { merged_at: 5 } })))).toBe("merged");
        expect(columnOf(task(4, "done", pr({ pr: { merged_at: 5, released_in: "v0.59.41" } })))).toBe("released");
        expect(stageWord(task(5, "review", { kind: "Spec" }))).toBe("spec");
        expect(stageWord(task(6, "review"))).toBe("build");
    });

    it("keeps a claimed task In progress while its holder works or idles", () => {
        expect(columnOf(task(1, "claimed", { holder: agent("Loap-3", "working") }))).toBe("progress");
        expect(columnOf(task(2, "claimed", { holder: agent("Loap-3", "idle") }))).toBe("progress");
    });

    it("moves it to Needs you when its holder asks, waits on a question, errors or is gone, and says why", () => {
        const asked = task(1, "blocked", { card: { question: "which branch?" } as KanbanTask["card"] });
        expect([columnOf(asked), needsReason(asked)]).toEqual(["needs", "asks: which branch?"]);
        const waiting = task(2, "claimed", { holder: agent("Loap-3", "waiting") });
        expect([columnOf(waiting), needsReason(waiting)]).toEqual(["needs", "waiting on you"]);
        expect(needsReason(task(3, "claimed", { holder: agent("Loap-3", "error") }))).toBe("agent errored");
        expect(needsReason(task(4, "claimed", { holder: null }))).toBe("agent closed");
        expect(needsReason(task(5, "failed", { attempts: 3 }))).toBe("failed after 3 attempts");
        expect(columnOf(task(5, "failed"))).toBe("needs");
    });
});

describe("a holder that can't work (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §4.3)", () => {
    const RESETS = new Date(2026, 9, 25, 19, 0).getTime();
    const limited = (over: Partial<KanbanTask> = {}) =>
        task(1, "claimed", {
            holder: agent("Naki-2", "limited"),
            claimed_by: "Naki-2",
            claimed_by_uid: "uid-Naki-2",
            holder_unavailable: { state: "limited", title: "Usage limit reached", resets_at_ms: RESETS, since_ms: 1 },
            ...over,
        });

    it("says out of quota until the reset, or just out of quota with none, and puts the card in Needs you", () => {
        const t = limited();
        expect(columnOf(t)).toBe("needs");
        expect(needsReason(t)).toMatch(/^out of quota until .*7:00/);
        expect(needsReason(t)).toBe(quotaReason(RESETS));
        const noTime = limited({ holder_unavailable: { state: "limited", title: "", since_ms: 1 } });
        expect(needsReason(noTime)).toBe("out of quota");
        expect(quotaReason(null)).toBe("out of quota");
    });

    it("says needs sign-in for a signed-out holder", () => {
        const t = task(2, "claimed", { holder: agent("Loap-3", "signed_out") });
        expect([columnOf(t), needsReason(t)]).toEqual(["needs", "needs sign-in"]);
    });

    it("leaves a question the holder asked as the reason, and a working holder alone", () => {
        const asked = task(3, "blocked", { holder: agent("Naki-2", "limited"), card: { question: "which branch?" } as KanbanTask["card"] });
        expect(needsReason(asked)).toBe("asks: which branch?");
        expect(needsReason(task(4, "claimed", { holder: agent("Naki-2", "working") }))).toBeNull();
    });

    it("counts a stuck task once: only when no open account episode covers its holder", () => {
        const t = limited();
        const other = task(5, "failed");
        expect(needsCountOf([t, other])).toBe(2);
        const byPane = { agent_ids: [], block_ids: ["b-Naki-2"] };
        const byUid = { agent_ids: ["uid-Naki-2"], block_ids: [] };
        const unrelated = { agent_ids: ["uid-someone"], block_ids: ["b-someone"] };
        expect(needsCountOf([t, other], [byPane])).toBe(1);
        expect(needsCountOf([t, other], [byUid])).toBe(1);
        expect(needsCountOf([t, other], [unrelated])).toBe(2);
        // Covered or not, it keeps its reason and sits in Needs you.
        expect(columnOf(t)).toBe("needs");
    });

    it("counts every other reason even when the holder's account is in an episode", () => {
        const asked = task(6, "blocked", { holder: agent("Naki-2", "limited"), claimed_by_uid: "uid-Naki-2" });
        const failed = task(7, "failed");
        expect(needsCountOf([asked, failed], [{ agent_ids: ["uid-Naki-2"], block_ids: ["b-Naki-2"] }])).toBe(2);
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
            [
                task(1, "review", { updated_at: 1000, kind: "spec" }),
                task(2, "review", { updated_at: 4000 }),
                task(3, "open"),
                task(4, "claimed", {
                    updated_at: 6000,
                    holder: agent("Naki-2", "working"),
                    card: { pr_url: "#4" } as KanbanTask["card"],
                }),
            ],
            10_000
        );
        expect(r).toEqual({ count: 3, specs: 1, oldestMs: 9000 });
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

    it("splits the board into lanes by repository, busy ones first, no repository last", () => {
        const at = (num: number, state: TaskState, repo: string) =>
            task(num, state, { card: { repo } as KanbanTask["card"] });
        const lanes = repoLanes([
            at(1, "done", "a/old"),
            at(2, "open", "z/busy"),
            at(3, "open", ""),
            at(4, "claimed", "b/busy"),
            at(5, "review", "z/busy"),
        ]);
        expect(lanes.map((l) => [l.label, l.tasks.length])).toEqual([
            ["b/busy", 1],
            ["z/busy", 2],
            ["a/old", 1],
            ["No repository", 1],
        ]);
        expect(repoLabel("local:C:/work/my-tool")).toBe("my-tool");
    });

    it("reads Review's soft limit, off unless a positive number", () => {
        expect(reviewLimitOf({ "kanban:reviewlimit": 4 })).toBe(4);
        expect(reviewLimitOf({ "kanban:reviewlimit": 0 })).toBe(0);
        expect(reviewLimitOf({ "kanban:reviewlimit": "4" })).toBe(0);
        expect(reviewLimitOf(undefined)).toBe(0);
    });

    it("says what a task still waits for, and reads the numbers typed for it", () => {
        const after = [
            { id: "a", num: 3, title: "a", state: "claimed" as const },
            { id: "b", num: 4, title: "b", state: "done" as const },
            { id: "c", num: 5, title: "c", state: "cancelled" as const },
            { id: "d", num: 6, title: "d", state: "failed" as const },
        ];
        const t = task(7, "open", { card: { after } as unknown as KanbanTask["card"] });
        expect(waitingOn(t).map((a) => a.num)).toEqual([3, 6]);
        expect(waitingOn(task(8, "open"))).toEqual([]);
        expect(parseAfter(" #12, 15;#3 ")).toEqual(["12", "15", "3"]);
        expect(parseAfter("")).toEqual([]);
    });

    it("labels a PR link with its number", () => {
        expect(prLabel("https://github.com/agentmuxai/agentmux/pull/5248")).toBe("#5248");
        expect(prLabel("https://example.com/merge_requests/3")).toBe("PR");
    });
});

describe("collapsed columns", () => {
    it("reads the pane's list as column ids, once each, in column order", () => {
        expect(parseCollapsed(["review", "backlog", "review", "nonsense", 7])).toEqual(["backlog", "review"]);
        expect(parseCollapsed(undefined)).toEqual([]);
        expect(parseCollapsed("review")).toEqual([]);
    });

    it("toggles a column in and out", () => {
        expect(toggleCollapsed([], "review")).toEqual(["review"]);
        expect(toggleCollapsed(["review", "backlog"], "review")).toEqual(["backlog"]);
        expect(toggleCollapsed(["review"], "backlog")).toEqual(["backlog", "review"]);
    });

    it("counts a column's tasks, the ones on hold and the oldest", () => {
        const tasks = [task(1, "held"), task(2, "open"), task(3, "held")];
        expect(columnStats(tasks, 10_000)).toEqual({ count: 3, held: 2, oldestMs: 9_000 });
        expect(columnStats(tasks, 10_000, 2).count).toBe(5);
        expect(columnStats([], 10_000, 1)).toEqual({ count: 1, held: 0, oldestMs: 0 });
    });

    it("takes a dropped card on the waiting columns only", () => {
        expect(takesDrops("backlog")).toBe(true);
        expect(takesDrops("specd")).toBe(true);
        expect(takesDrops("review")).toBe(false);
        expect(takesDrops("merged")).toBe(false);
    });
});
