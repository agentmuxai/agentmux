// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ReviewRow } from "@/types/rpc/ReviewRow";
import { describe, expect, it } from "vitest";
import { filterRows, groupRows, inTaskRef, kanbanIdOf, NO_FILTER, rowKey, taskRefOf } from "./reviews-model";

function row(over: Partial<ReviewRow> & { number: number }): ReviewRow {
    return {
        branch: "",
        reviews: [],
        merge: "open",
        released_in: "",
        waits_on: "reviewer",
        why: "no_review",
        updated_at_ms: 1000,
        ...over,
    } as ReviewRow;
}

const task = (num: number, title: string, state = "claimed") => ({ ref: `#${num}`, num, title, state });

describe("groupRows", () => {
    it("puts the pull requests that wait on a person first, longest-waiting first, and shows each once", () => {
        const rows = [
            row({ number: 1, task: task(1, "A"), waits_on: "person", why: "awaiting_accept", updated_at_ms: 500 }),
            row({ number: 2, task: task(2, "B"), waits_on: "person", why: "awaiting_accept", updated_at_ms: 100 }),
            row({ number: 3, task: task(1, "A"), updated_at_ms: 900 }),
        ];
        const groups = groupRows(rows);
        expect(groups.map((g) => g.key)).toEqual(["person", "task:#1"]);
        expect(groups[0].rows.map((r) => r.number)).toEqual([2, 1]);
        expect(groups[1].rows.map((r) => r.number)).toEqual([3]);
    });

    it("orders the tasks by their newest change and puts the rows with no task last", () => {
        const rows = [
            row({ number: 1, task: task(1, "old"), updated_at_ms: 100 }),
            row({ number: 2, task: task(2, "new"), updated_at_ms: 900 }),
            row({ number: 3, updated_at_ms: 2000 }),
        ];
        const groups = groupRows(rows);
        expect(groups.map((g) => g.key)).toEqual(["task:#2", "task:#1", "none"]);
        expect(groups[0].title).toBe("#2 new");
        expect(groups[0].state).toBe("claimed");
        expect(groups[2].kind).toBe("none");
    });

    it("keeps one task's pull requests together", () => {
        const rows = [row({ number: 1, task: task(5, "spec") }), row({ number: 2, task: task(5, "spec") })];
        const groups = groupRows(rows);
        expect(groups).toHaveLength(1);
        expect(groups[0].rows).toHaveLength(2);
    });

    it("keeps tasks with one number on two boards apart", () => {
        const rows = [
            row({ number: 1, task: task(7, "default board") }),
            row({ number: 2, task: { ref: "work#7", num: 7, title: "work board", state: "claimed" } }),
        ];
        const groups = groupRows(rows);
        expect(groups.map((g) => g.key).sort()).toEqual(["task:#7", "task:work#7"]);
        expect(groups.map((g) => g.rows.length)).toEqual([1, 1]);
    });

    it("has no group for no rows, and no 'Needs a person' when none waits on one", () => {
        expect(groupRows([])).toEqual([]);
        expect(groupRows([row({ number: 1, task: task(1, "A") })]).map((g) => g.kind)).toEqual(["task"]);
    });
});

describe("filterRows", () => {
    const rows = [
        row({
            number: 7407,
            task: task(2580, "Browser kinks"),
            agent: "manoz",
            repo: "agentmuxai/agentmux",
            branch: "manoz/browser",
        }),
        row({
            number: 12,
            task: task(9, "Spec"),
            agent: "lazo",
            repo: "agentmuxai/muxreview",
            waits_on: "author",
            why: "checks_failing",
        }),
    ];

    it("keeps everything for no filter", () => {
        expect(filterRows(rows, NO_FILTER)).toHaveLength(2);
    });

    it("needs every word to match, over number, task, agent, repository, branch and why", () => {
        const count = (text: string) => filterRows(rows, { text, waitsOn: null }).length;
        expect(count("7407")).toBe(1);
        expect(count("browser manoz")).toBe(1);
        expect(count("browser lazo")).toBe(0);
        expect(count("muxreview")).toBe(1);
        expect(count("checks failing")).toBe(1);
        expect(count("  ")).toBe(2);
    });

    it("narrows by what it waits on", () => {
        expect(filterRows(rows, { text: "", waitsOn: "author" }).map((r) => r.number)).toEqual([12]);
        expect(filterRows(rows, { text: "", waitsOn: "person" })).toEqual([]);
    });
});

describe("task refs", () => {
    const onWork = row({ number: 2, task: { ref: "work#7", num: 7, title: "x", state: "claimed" } });
    const onDefault = row({ number: 1, task: task(7, "y") });

    it("reads a bare number or #n as the default board's task, and a board ref as it is", () => {
        expect(taskRefOf("7")).toBe("#7");
        expect(taskRefOf(" #7 ")).toBe("#7");
        expect(taskRefOf("Work#7")).toBe("work#7");
    });

    it("never matches another board's task by its bare number", () => {
        expect(inTaskRef(onDefault, "7")).toBe(true);
        expect(inTaskRef(onWork, "7")).toBe(false);
        expect(inTaskRef(onWork, "work#7")).toBe(true);
        expect(inTaskRef(row({ number: 3 }), "7")).toBe(false);
    });

    it("opens the Kanban by the ref without its leading #", () => {
        expect(kanbanIdOf("#12")).toBe("12");
        expect(kanbanIdOf("work#12")).toBe("work#12");
    });
});

describe("text helpers", () => {
    it("names a row by repository and number, or by the repository id when the repository is unknown", () => {
        expect(rowKey(row({ number: 5, repo: "o/r" }))).toBe("o/r#5");
        expect(rowKey(row({ number: 5, repo_id: 9 }))).toBe("id:9#5");
    });
});
