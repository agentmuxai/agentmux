// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { IDLE_ACTIVITY, type ActivityState } from "@/app/store/agent-pane-state/types";
import { presentStatus, RANK, statusCandidates, TIMING, type StatusInput, type StatusMemory } from "./present-status";
import { toolActivity } from "./tool-labels";

const T0 = 1_000_000;
const tool = (name: string, params: Record<string, unknown>, startedAt: number, id = name) => ({
    id,
    activity: toolActivity(name, params),
    startedAt,
});
const input = (over: Partial<StatusInput> = {}): StatusInput => ({
    nowMs: T0,
    needsYou: null,
    held: null,
    leadIn: null,
    activity: IDLE_ACTIVITY,
    turnStartedAt: T0 - 60_000,
    goal: "Fix the login redirect loop",
    phrase: "Working",
    ...over,
});
const act = (over: Partial<ActivityState>): ActivityState => ({ ...IDLE_ACTIVITY, ...over });

/** Run the presenter over a timeline, as the row's tick does. */
function play(frames: Array<Partial<StatusInput>>): string[] {
    let memory: StatusMemory | null = null;
    return frames.map((f) => {
        const r = presentStatus(input(f), memory);
        memory = r.memory;
        return r.line.text;
    });
}

describe("statusCandidates: what is eligible", () => {
    it("with nothing going on, the goal, then the phrase", () => {
        expect(statusCandidates(input()).map((l) => l.text)).toEqual(["Fix the login redirect loop", "Working…"]);
        expect(statusCandidates(input({ goal: null }))[0].text).toBe("Working…");
    });

    it("a quick tool call never shows; one that runs on does, with its time once long", () => {
        const bash = tool("Bash", { description: "Run the srv test suite" }, T0 - 1_000);
        expect(statusCandidates(input({ activity: act({ tools: [bash] }) }))[0].rank).toBe(RANK.goal);
        expect(statusCandidates(input({ activity: act({ tools: [{ ...bash, startedAt: T0 - 2_000 }] }) }))[0].text).toBe(
            "Running the srv test suite",
        );
        expect(statusCandidates(input({ activity: act({ tools: [{ ...bash, startedAt: T0 - 72_000 }] }) }))[0].text).toBe(
            "Running the srv test suite · 1m 12s",
        );
    });

    it("folds parallel calls of one kind", () => {
        const reads = ["a.ts", "b.ts", "c.ts"].map((f, i) => tool("Read", { file_path: f }, T0 - 3_000, `r${i}`));
        expect(statusCandidates(input({ activity: act({ tools: reads }) }))[0].text).toBe("Reading 3 files");
    });

    it("thinking, writing and composing each show past their own threshold", () => {
        const at = (phase: ActivityState["phase"], age: number) =>
            statusCandidates(input({ activity: act({ phase, phaseSince: T0 - age }) }))[0].text;
        expect(at("thinking", TIMING.thinkingPromoteMs - 1)).toBe("Fix the login redirect loop");
        expect(at("thinking", TIMING.thinkingPromoteMs)).toBe("Thinking");
        expect(at("writing", TIMING.writingPromoteMs)).toBe("Writing the reply");
        expect(at("composing", TIMING.composingPromoteMs)).toBe("Preparing the next step");
    });

    it("a long wait for the model is said, with its time", () => {
        const a = act({ phase: "requesting", phaseSince: T0 - 34_000 });
        expect(statusCandidates(input({ activity: a }))[0]).toMatchObject({ rank: RANK.anomaly, text: "Waiting on the model · 34s" });
        expect(statusCandidates(input({ activity: act({ phase: "requesting", phaseSince: T0 - 5_000 }) }))[0].rank).toBe(RANK.goal);
    });

    it("the plan's step, only when set this turn", () => {
        const plan = { activeForm: "Writing the spec", index: 3, total: 7 };
        expect(statusCandidates(input({ activity: act({ plan, planAt: T0 - 1_000 }) }))[0].text).toBe("Writing the spec (3/7)");
        expect(statusCandidates(input({ activity: act({ plan, planAt: T0 - 120_000 }) }))[0].text).toBe("Fix the login redirect loop");
    });

    it("needs-you beats everything; a held status beats activity; a lead-in beats activity", () => {
        const busy = act({ tools: [tool("Bash", { description: "Run it" }, T0 - 5_000)] });
        expect(statusCandidates(input({ activity: busy, needsYou: "Waiting for your approval: git push", held: "Stopping…" }))[0].text).toBe(
            "Waiting for your approval: git push",
        );
        expect(statusCandidates(input({ activity: busy, held: "Compacting…" }))[0].text).toBe("Compacting…");
        expect(statusCandidates(input({ activity: busy, leadIn: "↳ jekt from AgentX" }))[0].text).toBe("↳ jekt from AgentX");
    });
});

describe("presentStatus: when to change", () => {
    const bashAt = (startedAt: number) => act({ tools: [tool("Bash", { description: "Run the tests" }, startedAt)] });

    it("a new line waits out the dwell of the one on screen, unless it ranks higher", () => {
        // The goal shows; a moment later the plan (rank 5) becomes eligible: it
        // ranks higher than the goal, so it comes at once.
        const plan = { activeForm: "Writing the spec", index: 1, total: 2 };
        expect(play([{}, { nowMs: T0 + 100, activity: act({ plan, planAt: T0 }) }])).toEqual([
            "Fix the login redirect loop",
            "Writing the spec (1/2)",
        ]);
    });

    it("a rank-4 line lingers briefly after its call ends, then the row falls back", () => {
        const frames = [
            { nowMs: T0, activity: bashAt(T0 - 2_000) },
            { nowMs: T0 + 500, activity: IDLE_ACTIVITY }, // the call ended
            { nowMs: T0 + 1_500, activity: IDLE_ACTIVITY },
            { nowMs: T0 + 2_600, activity: IDLE_ACTIVITY },
        ];
        expect(play(frames)).toEqual(["Running the tests", "Running the tests", "Running the tests", "Fix the login redirect loop"]);
    });

    it("between two calls the line holds, then names the next call once it runs long enough", () => {
        const next = act({ tools: [tool("Read", { file_path: "a.ts" }, T0 + 600)] });
        const frames = [
            { nowMs: T0, activity: bashAt(T0 - 2_000) },
            { nowMs: T0 + 700, activity: next }, // next call too young to show: hold
            { nowMs: T0 + 2_200, activity: next }, // old enough now
        ];
        expect(play(frames)).toEqual(["Running the tests", "Running the tests", "Reading a.ts"]);
    });

    it("needs-you preempts at once, with no dwell", () => {
        const frames = [{ nowMs: T0, activity: bashAt(T0 - 2_000) }, { nowMs: T0 + 100, activity: bashAt(T0 - 2_000), needsYou: "Question for you" }];
        expect(play(frames)).toEqual(["Running the tests", "Question for you"]);
    });

    it("a counter moving keeps the line's key, so the row doesn't type it out again", () => {
        let memory: StatusMemory | null = null;
        const a = presentStatus(input({ nowMs: T0, activity: bashAt(T0 - 11_000) }), memory);
        memory = a.memory;
        const b = presentStatus(input({ nowMs: T0 + 1_000, activity: bashAt(T0 - 11_000) }), memory);
        expect(a.line.text).not.toBe(b.line.text);
        expect(a.line.key).toBe(b.line.key);
    });
});
