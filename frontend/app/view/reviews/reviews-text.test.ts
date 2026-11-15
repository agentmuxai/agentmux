// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ReviewsVerdict } from "@/types/rpc/ReviewsVerdict";
import type { WaitsOn } from "@/types/rpc/WaitsOn";
import { describe, expect, it } from "vitest";
import {
    ageText,
    byText,
    checksText,
    groupTitle,
    mergeText,
    verdictText,
    waitsLabel,
    waitsOptions,
    waitsShort,
    whyText,
} from "./reviews-text";

const WAITS: WaitsOn[] = ["person", "author", "reviewer", "ci", "queue", "none", "unknown"];

describe("the pane's words", () => {
    it("names every waits-on, in a sentence and in one word", () => {
        expect(WAITS.map(waitsLabel)).toEqual([
            "Waits on a person",
            "Waits on its author",
            "Waits on a reviewer",
            "Waits on checks",
            "In the merge queue",
            "Nothing to do",
            "Unknown",
        ]);
        expect(WAITS.map(waitsShort)).toEqual(["person", "author", "reviewer", "checks", "queue", "done", "unknown"]);
    });

    it("says every reason srv gives in words, and a reason it does not know as its own words", () => {
        const codes = [
            "changes_requested",
            "checks_failing",
            "checks_running",
            "no_review",
            "review_stale",
            "awaiting_accept",
            "ready_to_merge",
            "merged",
            "closed",
            "task_finished",
            "queued",
            "unknown",
        ];
        for (const code of codes) expect(whyText(code), code).not.toMatch(/^reviews\./);
        expect(whyText("review_stale")).toBe("The review is of an older commit");
        expect(whyText("some_new_reason")).toBe("some new reason");
    });

    it("says a merge, checks or verdict state, and a state it does not know as its words", () => {
        expect([mergeText("open"), mergeText("merged"), mergeText("closed"), mergeText("in_queue")]).toEqual([
            "open",
            "merged",
            "closed",
            "in queue",
        ]);
        expect([checksText("passing"), checksText("failing"), checksText("pending"), checksText("neutral")]).toEqual([
            "passing",
            "failing",
            "pending",
            "neutral",
        ]);
        const v = (state: string, stale = false): ReviewsVerdict =>
            ({ by: "bot", state, at_ms: 1, stale }) as ReviewsVerdict;
        expect(verdictText(v("approved"))).toBe("approved");
        expect(verdictText(v("changes_requested"))).toBe("changes requested");
        expect(verdictText(v("approved", true))).toBe("approved, older commit");
        expect(verdictText(v("dismissed_by_owner"))).toBe("dismissed by owner");
        expect([byText("bot"), byText("codex"), byText("person")]).toEqual(["Review bot", "Codex", "Person"]);
    });

    it("heads the two fixed groups with words and a task's group with its own title", () => {
        expect(groupTitle({ key: "person", kind: "person", title: "", rows: [] })).toBe("Needs a person");
        expect(groupTitle({ key: "none", kind: "none", title: "", rows: [] })).toBe("No task");
        expect(groupTitle({ key: "task:#2", kind: "task", title: "#2 Fix it", rows: [] })).toBe("#2 Fix it");
    });

    it("gives ages in minutes, hours and days", () => {
        const now = 10_000_000_000;
        expect(ageText(now - 5_000, now)).toBe("now");
        expect(ageText(now - 3 * 60_000, now)).toBe("3m");
        expect(ageText(now - 5 * 3_600_000, now)).toBe("5h");
        expect(ageText(now - 3 * 86_400_000, now)).toBe("3d");
        expect(ageText(now + 5_000, now)).toBe("now");
    });

    it("lists the waits-on switch with All first", () => {
        expect(waitsOptions().map((o) => o.value)).toEqual([null, "person", "author", "reviewer", "ci"]);
        expect(waitsOptions().map((o) => o.label)).toEqual(["All", "Person", "Author", "Reviewer", "Checks"]);
    });
});
