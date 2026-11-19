// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban's stages: where each task lands, the rules a set must keep (the same as srv's), and the edits the
// editor makes to its draft.

import { describe, expect, it } from "vitest";

import { columnOf } from "./kanban-columns";
import {
    addStage,
    canRemove,
    moveStage,
    moveStageNear,
    recolourStage,
    removalTarget,
    removeStage,
    renameStage,
    sameStages,
    toggleSource,
} from "./kanban-stages-draft";
import {
    defaultStages,
    parseCollapsedStages,
    stageCounts,
    stageOfTask,
    stagesOfBoard,
    stageTasks,
    toggleCollapsedStage,
    toStageSet,
    validateStages,
    isFinishedStage,
    isWaitingStage,
    type KanbanStage,
} from "./kanban-stages";
import type { KanbanTask } from "./kanban-types";

function task(over: Partial<KanbanTask> & { card?: Partial<KanbanTask["card"]> }): KanbanTask {
    const { card, ...rest } = over;
    return {
        id: "t",
        title: "t",
        kind: "",
        state: "open",
        priority: 0,
        attempts: 0,
        created_at: 1,
        updated_at: 1,
        target_agent: "",
        claimed_by: "",
        card: { num: 1, pr_url: "", after: [], ...card },
        ...rest,
    } as unknown as KanbanTask;
}

/** Backlog and Spec'd together, In Research and In progress together as Doing, Merged and Released as Done. */
function merged(): KanbanStage[] {
    let s = defaultStages();
    s = removeStage(s, "specd", "backlog");
    s = removeStage(s, "research", "progress");
    s = renameStage(s, "progress", "Doing");
    s = removeStage(s, "released", "merged");
    return renameStage(s, "merged", "Done");
}

describe("the default stages", () => {
    it("are today's eight columns, valid, one per source", () => {
        const s = defaultStages();
        expect(s.map((x) => x.id)).toEqual(["backlog", "research", "specd", "progress", "needs", "review", "merged", "released"]);
        expect(s.every((x) => x.takes.length === 1 && x.takes[0] === x.id)).toBe(true);
        expect(validateStages(s)).toEqual([]);
    });

    it("put every task where it sat before", () => {
        const s = defaultStages();
        for (const t of [task({}), task({ state: "claimed", claimed_by: "a" }), task({ state: "failed" }), task({ state: "done" })]) {
            expect(stageOfTask(t, s).id).toBe(columnOf(t));
        }
    });
});

describe("merged stages", () => {
    it("are valid, and a task lands in the stage that shows its source", () => {
        const s = merged();
        expect(validateStages(s)).toEqual([]);
        expect(s.map((x) => x.name)).toEqual(["Backlog", "Doing", "Needs you", "Review", "Done"]);
        expect(stageOfTask(task({ state: "done" }), s).name).toBe("Done");
        expect(stageOfTask(task({ state: "claimed", claimed_by: "a", kind: "spec", holder: { state: "working" } } as never), s).name).toBe("Doing");
        expect(stageOfTask(task({ state: "blocked" }), s).name).toBe("Needs you");
        expect(stageCounts([task({}), task({ state: "done" }), task({ state: "done" })], s)).toMatchObject({ backlog: 1, merged: 2 });
    });

    it("sort a waiting stage by backlog order and the rest newest first", () => {
        const s = merged();
        const a = task({ id: "a", priority: 0, created_at: 5 });
        const b = task({ id: "b", priority: 5, created_at: 9 });
        expect(stageTasks([a, b], s[0]).map((t) => t.id)).toEqual(["b", "a"]);
        const x = task({ id: "x", state: "done", updated_at: 1 });
        const y = task({ id: "y", state: "done", updated_at: 9 });
        expect(stageTasks([x, y], s[4]).map((t) => t.id)).toEqual(["y", "x"]);
    });

    it("know which stages are waiting and which finished", () => {
        const s = merged();
        expect(s.map(isWaitingStage)).toEqual([true, false, false, false, false]);
        expect(s.map(isFinishedStage)).toEqual([false, false, false, false, true]);
    });
});

describe("validateStages", () => {
    const why = (s: KanbanStage[]) => validateStages(s).map((p) => p.message);

    it("needs two to eight stages", () => {
        expect(why(defaultStages().slice(0, 1)).join()).toContain("at least 2");
        expect(why([...defaultStages(), { id: "x", name: "Extra", colour: "grey", takes: [] }]).join()).toContain("at most 8");
    });

    it("needs every source shown by exactly one stage", () => {
        const s = defaultStages();
        s[0] = { ...s[0], takes: ["backlog", "review"] };
        expect(why(s).join()).toContain("Review is shown by another stage too");
        const t = merged();
        t[1] = { ...t[1], takes: ["progress"] };
        expect(why(t).join()).toContain("No stage shows In Research");
    });

    it("keeps Needs you to a stage of its own", () => {
        const s = merged();
        s[2] = { ...s[2], takes: ["needs", "review"] };
        s[3] = { ...s[3], takes: [] };
        expect(why(s).join()).toContain("shows only what needs you");
    });

    it("refuses empty, duplicate, long and reserved names", () => {
        let s = renameStage(defaultStages(), "backlog", "  ");
        expect(why(s)).toContain("Give it a name");
        s = renameStage(defaultStages(), "backlog", "merged");
        expect(why(s)).toContain("Another stage has this name");
        s = renameStage(defaultStages(), "backlog", "x".repeat(25));
        expect(why(s).join()).toContain("at most 24");
        s = renameStage(renameStage(defaultStages(), "needs", "Ask me"), "backlog", "needs you");
        expect(why(s).join()).toContain("name of the stage that shows what needs you");
        expect(why(renameStage(defaultStages(), "needs", "Ask me"))).toEqual([]);
        expect(why(renameStage(defaultStages(), "backlog", "完了 ✅"))).toEqual([]);
    });

    it("asks a new stage to show something", () => {
        expect(why(addStage(defaultStages().slice(0, 7).concat([]), "New", "st-aaaaaa")).join()).toContain("Pick at least one thing");
    });
});

describe("the draft edits", () => {
    it("rename, recolour and move", () => {
        const s = defaultStages();
        expect(renameStage(s, "review", "Checking")[5].name).toBe("Checking");
        expect(recolourStage(s, "review", "blue")[5].colour).toBe("blue");
        expect(moveStage(s, "review", -1).map((x) => x.id).slice(3, 7)).toEqual(["progress", "review", "needs", "merged"]);
        expect(moveStage(s, "backlog", -1)).toEqual(s);
        expect(moveStage(s, "released", 3)).toEqual(s);
    });

    it("moves a stage next to another, before or after, and ignores a drop on itself or on nothing", () => {
        const s = defaultStages();
        const ids = (x: typeof s) => x.map((y) => y.id);
        expect(ids(moveStageNear(s, "review", "backlog", false)).slice(0, 2)).toEqual(["review", "backlog"]);
        expect(ids(moveStageNear(s, "backlog", "review", true)).slice(3, 6)).toEqual(["needs", "review", "backlog"]);
        expect(ids(moveStageNear(s, "backlog", "review", false)).slice(3, 6)).toEqual(["needs", "backlog", "review"]);
        expect(moveStageNear(s, "review", "review", true)).toEqual(s);
        expect(moveStageNear(s, "review", "nope", true)).toEqual(s);
        expect(moveStageNear(s, "nope", "review", true)).toEqual(s);
        expect(moveStageNear(s, "review", "review", false)).not.toBe(s);
    });

    it("turning a source on takes it from the stage that had it", () => {
        const s = toggleSource(defaultStages(), "backlog", "specd", true);
        expect(s[0].takes).toEqual(["backlog", "specd"]);
        expect(s[2].takes).toEqual([]);
        expect(validateStages(s).map((p) => p.message)).toContain("Pick at least one thing for it to show");
    });

    it("keeps Needs you to its own source", () => {
        const s = defaultStages();
        expect(toggleSource(s, "backlog", "needs", true)).toEqual(s);
        expect(toggleSource(s, "needs", "review", true)).toEqual(s);
    });

    it("adds a stage with a free colour, up to eight, and a name is the caller's", () => {
        const s = removeStage(defaultStages(), "released", "merged");
        const added = addStage(s, "Parked", "st-abc123");
        expect(added).toHaveLength(8);
        expect(added[7]).toMatchObject({ id: "st-abc123", name: "Parked", takes: [] });
        expect(addStage(added, "More", "st-zzzzzz")).toHaveLength(8);
    });

    it("removes a stage into the one before it (the next for the first), never into Needs you", () => {
        const s = defaultStages();
        expect(removalTarget(s, "review")).toBe("progress");
        expect(removalTarget(s, "backlog")).toBe("research");
        expect(removalTarget(addStage(s.slice(0, 7), "New", "st-aaaaaa"), "st-aaaaaa")).toBeNull();
        expect(removeStage(s, "review", "progress").find((x) => x.id === "progress")?.takes).toEqual(["progress", "review"]);
    });

    it("never removes the stage that shows what needs the operator", () => {
        const s = defaultStages();
        expect(canRemove(s, "needs")).toBe(false);
        expect(removeStage(s, "needs", "backlog")).toEqual(s);
    });

    it("compares drafts by what the person can change", () => {
        expect(sameStages(defaultStages(), defaultStages())).toBe(true);
        expect(sameStages(defaultStages(), renameStage(defaultStages(), "review", "X"))).toBe(false);
    });
});

describe("stagesOfBoard", () => {
    it("is the default eight with nothing stored, with an error, or with a version it doesn't know", () => {
        expect(stagesOfBoard(undefined)).toEqual(defaultStages());
        expect(stagesOfBoard({ stages: toStageSet(merged()), stages_error: "broken" })).toEqual(defaultStages());
        expect(stagesOfBoard({ stages: { version: 2 as unknown as 1, stages: [] } })).toEqual(defaultStages());
    });

    it("is the stored set when it is whole, with an unknown colour as grey and an unknown source dropped", () => {
        const set = toStageSet(merged());
        expect(stagesOfBoard({ stages: set })).toEqual(merged());
        const odd = toStageSet(merged());
        (odd.stages[0] as { colour: string }).colour = "chartreuse";
        (odd.stages[0].takes as string[]).push("future");
        const got = stagesOfBoard({ stages: odd });
        expect(got[0].colour).toBe("grey");
        expect(got[0].takes).toEqual(["backlog", "specd"]);
    });

    it("falls back to the default when the stored set breaks a rule", () => {
        const set = toStageSet(merged());
        set.stages[1].takes = [];
        expect(stagesOfBoard({ stages: set })).toEqual(defaultStages());
    });
});

describe("collapsed stages", () => {
    it("reads only the shown board's stages, in stage order", () => {
        const s = merged();
        expect(parseCollapsedStages(["merged", "backlog", "released", 4], s)).toEqual(["backlog", "merged"]);
        expect(parseCollapsedStages("x", s)).toEqual([]);
    });

    it("toggles one and keeps the ids another board's stages have stored", () => {
        const s = merged();
        expect(toggleCollapsedStage(["backlog", "st-other"], s, "merged")).toEqual(["st-other", "backlog", "merged"]);
        expect(toggleCollapsedStage(["backlog", "st-other"], s, "backlog")).toEqual(["st-other"]);
    });
});
