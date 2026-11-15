// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ReviewRow } from "@/types/rpc/ReviewRow";
import { describe, expect, it } from "vitest";
import { ageText, filterRows, groupRows, NO_FILTER, rowKey, whyText } from "./reviews-model";

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
        expect(groups.map((g) => g.key)).toEqual(["person", "task:1"]);
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
        expect(groups.map((g) => g.key)).toEqual(["task:2", "task:1", "none"]);
        expect(groups[0].title).toBe("#2 new");
        expect(groups[0].state).toBe("claimed");
        expect(groups[2].title).toBe("No task");
    });

    it("keeps one task's pull requests together", () => {
        const rows = [row({ number: 1, task: task(5, "spec") }), row({ number: 2, task: task(5, "spec") })];
        const groups = groupRows(rows);
        expect(groups).toHaveLength(1);
        expect(groups[0].rows).toHaveLength(2);
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

describe("text helpers", () => {
    it("names a row by repository and number, or by the repository id when the repository is unknown", () => {
        expect(rowKey(row({ number: 5, repo: "o/r" }))).toBe("o/r#5");
        expect(rowKey(row({ number: 5, repo_id: 9 }))).toBe("id:9#5");
    });

    it("says why in words, and a code it does not know as its own words", () => {
        expect(whyText("review_stale")).toBe("The review is of an older commit");
        expect(whyText("some_new_reason")).toBe("some new reason");
    });

    it("gives ages in minutes, hours and days", () => {
        const now = 10_000_000_000;
        expect(ageText(now - 5_000, now)).toBe("now");
        expect(ageText(now - 3 * 60_000, now)).toBe("3m");
        expect(ageText(now - 5 * 3_600_000, now)).toBe("5h");
        expect(ageText(now - 3 * 86_400_000, now)).toBe("3d");
        expect(ageText(now + 5_000, now)).toBe("now");
    });
});
