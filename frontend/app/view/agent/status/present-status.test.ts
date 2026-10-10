// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { IDLE_ACTIVITY, type ActivityState } from "@/app/store/agent-pane-state/types";
import { presentStatus, RANK, slowRequestMs, statusCandidates, TIMING, type StatusInput, type StatusMemory } from "./present-status";
import { toolActivity } from "@/app/store/agent-pane-state/tool-labels";

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

    // The row changed too often: a line was replaced the moment it finished
    // typing out, because the dwell ran from when it was chosen, not from
    // when it was readable.
    it("the dwell runs from the end of the type-out, not from when the line was chosen", () => {
        const long = act({ tools: [tool("Bash", { description: "Run the full srv integration test suite now" }, T0)] });
        const both = act({
            tools: [
                tool("Bash", { description: "Run the full srv integration test suite now" }, T0),
                tool("Read", { file_path: "a.ts" }, T0 + 100, "r"),
            ],
        });
        // 47 characters at revealCharMs: fully on screen ~1.3 s after it is chosen.
        const shownAt = T0 + TIMING.toolPromoteMs;
        const readyAt = shownAt + "Running the full srv integration test suite now".length * TIMING.revealCharMs;
        const frames = [
            { nowMs: T0, activity: long },
            { nowMs: shownAt, activity: long },
            // Past the old dwell (from shownAt), still inside the new one (from readyAt).
            { nowMs: shownAt + TIMING.dwellMs + 100, activity: both },
            { nowMs: readyAt + TIMING.dwellMs, activity: both },
        ];
        expect(play(frames)).toEqual([
            "Fix the login redirect loop",
            "Running the full srv integration test suite now",
            "Running the full srv integration test suite now",
            "2 tools running",
        ]);
    });

    it("types out only when the rank changes; within a rank the line swaps in at once", () => {
        let memory: StatusMemory | null = null;
        const step = (f: Partial<StatusInput>) => {
            const r = presentStatus(input(f), memory);
            memory = r.memory;
            return r.line;
        };
        const read = (file: string, id: string, at: number) => tool("Read", { file_path: file }, at, id);
        expect(step({ nowMs: T0 }).reveal).toBe(true); // the goal
        const a = step({ nowMs: T0 + 2_000, activity: act({ tools: [read("a.ts", "a", T0)] }) });
        expect([a.text, a.reveal]).toEqual(["Reading a.ts", true]); // goal → now
        const ab = step({ nowMs: T0 + 6_000, activity: act({ tools: [read("a.ts", "a", T0), read("b.ts", "b", T0 + 100)] }) });
        expect([ab.text, ab.reveal]).toEqual(["Reading 2 files", false]); // now → now
        const q = step({ nowMs: T0 + 6_100, activity: act({ tools: [read("a.ts", "a", T0)] }), needsYou: "Waiting for your answer" });
        expect(q.reveal).toBe(true); // now → needs you
    });

    it("after a call ends, the line holds while the next call is still too young to show", () => {
        const next = act({ tools: [tool("Read", { file_path: "a.ts" }, T0 + 1_800)] });
        const frames = [
            { nowMs: T0, activity: bashAt(T0 - 2_000) },
            { nowMs: T0 + 100, activity: IDLE_ACTIVITY }, // the call ended
            { nowMs: T0 + 1_900, activity: next }, // the next began at +1.8 s
            { nowMs: T0 + TIMING.holdMs + 100, activity: next }, // HOLD is over, the next is 300 ms old
            { nowMs: T0 + 3_400, activity: next }, // old enough now
        ];
        expect(play(frames)).toEqual([
            "Running the tests",
            "Running the tests",
            "Running the tests",
            "Running the tests", // not a flash of the goal
            "Reading a.ts",
        ]);
    });

    it("after a call ends, the line also holds while the model writes the next call's input", () => {
        const composing = act({ phase: "composing", phaseSince: T0 + 1_500 });
        const frames = [
            { nowMs: T0, activity: bashAt(T0 - 2_000) },
            { nowMs: T0 + 100, activity: IDLE_ACTIVITY },
            { nowMs: T0 + TIMING.holdMs + 100, activity: composing }, // HOLD over; composing 600 ms
            { nowMs: T0 + 1_500 + TIMING.composingPromoteMs, activity: composing }, // now it shows
        ];
        expect(play(frames)).toEqual(["Running the tests", "Running the tests", "Running the tests", "Preparing the next step"]);
    });

    it("a todo-list update is not a next call warming up, so the row falls back", () => {
        const todo = act({ tools: [tool("TodoWrite", { todos: [] }, T0 + 1_800)] });
        expect(todo.tools[0].activity.family).toBe("plan");
        const frames = [
            { nowMs: T0, activity: bashAt(T0 - 2_000) },
            { nowMs: T0 + 100, activity: IDLE_ACTIVITY },
            { nowMs: T0 + TIMING.holdMs + 100, activity: todo },
        ];
        expect(play(frames).at(-1)).toBe("Fix the login redirect loop");
    });

    it("a status line goes the moment its cause does, without waiting out a dwell", () => {
        // Answered: the question line must not linger.
        const asked = play([
            { nowMs: T0, activity: bashAt(T0 - 2_000) },
            { nowMs: T0 + 100, activity: bashAt(T0 - 2_000), needsYou: "Waiting for your answer" },
            { nowMs: T0 + 200, activity: bashAt(T0 - 2_000) },
        ]);
        expect(asked).toEqual(["Running the tests", "Waiting for your answer", "Running the tests"]);
        // The model answered: "Waiting on the model" must not stay, frozen.
        const waiting = act({ phase: "requesting", phaseSince: T0 - TIMING.slowRequestMs - 1_000 });
        const answered = act({ phase: "responding", phaseSince: T0 + 100 });
        const lines = play([{ nowMs: T0, activity: waiting }, { nowMs: T0 + 100, activity: answered }]);
        expect(lines[0]).toMatch(/^Waiting on the model/);
        expect(lines[1]).not.toMatch(/^Waiting on the model/);
    });

    it("with no type-out (reduced motion, a turn's first line), the dwell starts at once", () => {
        const plan = (i: number) => act({ plan: { activeForm: "Tracing the session cookie through the redirect", index: i, total: 5 }, planAt: T0 });
        const frames = (instantReveal: boolean) => [
            { nowMs: T0, activity: plan(1), instantReveal },
            { nowMs: T0 + TIMING.dwellMs + 100, activity: plan(2), instantReveal },
        ];
        // Typed: 54 characters take ~1.5 s, so the next step waits.
        expect(play(frames(false))[1]).toBe("Tracing the session cookie through the redirect (1/5)");
        // Shown at once: its dwell is over, so the next step comes.
        expect(play(frames(true))[1]).toBe("Tracing the session cookie through the redirect (2/5)");
    });

    it("a line that grows while it types out is ready only once its new end is typed", () => {
        const agent = (steps: string[]) =>
            act({
                tools: [
                    tool("Task", { description: "map it", subagent_type: "Explore" }, T0 - 2_000, "ag"),
                    ...steps.map((f, i) => ({ ...tool("Read", { file_path: f }, T0, `s${i}`), parentId: "ag" })),
                ],
            });
        let memory: StatusMemory | null = null;
        const step = (f: Partial<StatusInput>) => {
            const r = presentStatus(input(f), memory);
            memory = r.memory;
            return r;
        };
        step({ nowMs: T0 - 100 }); // the goal
        const short = step({ nowMs: T0, activity: agent([]) });
        const readyShort = short.memory.readyAt;
        const grown = step({ nowMs: T0 + 200, activity: agent(["a_rather_long_module_name.ts"]) });
        expect(grown.line.key).toBe(short.line.key);
        expect(grown.memory.readyAt).toBe(T0 + grown.line.text.length * TIMING.revealCharMs);
        expect(grown.memory.readyAt).toBeGreaterThan(readyShort);
    });

    it("a burst of quick calls after a line ended can't keep that line up past the cap", () => {
        // Each call ends before it could show, the next starts at once: always
        // "warming". Capped at HOLD plus the longest promote threshold.
        const burst = (now: number) => act({ tools: [tool("Read", { file_path: "x.ts" }, now - 200, `q${Math.floor(now / 1_000)}`)] });
        const cap = T0 + TIMING.holdMs + Math.max(TIMING.toolPromoteMs, TIMING.composingPromoteMs);
        const frames = [
            { nowMs: T0, activity: bashAt(T0 - 2_000) },
            { nowMs: T0 + 100, activity: burst(T0 + 100) },
            { nowMs: cap - 100, activity: burst(cap - 100) },
            { nowMs: cap + 100, activity: burst(cap + 100) },
        ];
        expect(play(frames)).toEqual(["Running the tests", "Running the tests", "Running the tests", "Fix the login redirect loop"]);
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

describe("phase 3b: subagent steps, test progress, the thinking headline, the goal beside", () => {
    it("a subagent's own call shows under its Agent call, not as a call of its own", () => {
        const agent = tool("Agent", { subagent_type: "Explore", description: "map the code" }, T0 - 5_000, "a1");
        const step = { ...tool("Read", { file_path: "/x/health.rs" }, T0 - 300, "s1"), parentId: "a1" };
        const l = statusCandidates(input({ activity: act({ tools: [agent, step] }) }))[0];
        expect(l.text).toBe("Explore agent: map the code · Reading health.rs");
        expect(l.key).toBe(statusCandidates(input({ activity: act({ tools: [agent] }) }))[0].key); // a new step never re-types
    });

    it("a test run's progress, in place of its time", () => {
        const bash = { ...tool("Bash", { description: "Run the tests" }, T0 - 30_000), progress: "41/123" };
        expect(statusCandidates(input({ activity: act({ tools: [bash] }) }))[0].text).toBe("Running the tests · 41/123");
    });

    it("thinking names what it is about once the model has said", () => {
        const a = act({ phase: "thinking", phaseSince: T0 - 5_000, thinkingHeadline: "Weighing the join rule" });
        expect(statusCandidates(input({ activity: a }))[0].text).toBe("Thinking: Weighing the join rule");
    });

    it("the goal is never added after another line; it shows only as a line of its own", () => {
        const busy = act({ tools: [tool("Bash", { description: "Run it" }, T0 - 5_000)] });
        expect(presentStatus(input({ activity: busy }), null).line).not.toHaveProperty("detail");
        expect(presentStatus(input({ activity: busy }), null).line.text).toBe("Running it");
        expect(presentStatus(input(), null).line.text).toBe("Fix the login redirect loop");
    });
});

describe("a command gone quiet", () => {
    it("says so once it wrote output and then nothing for a minute", () => {
        const bash = (outputAt?: number) => ({ ...tool("Bash", { description: "Run the build" }, T0 - 300_000, "b1"), outputAt });
        expect(statusCandidates(input({ activity: act({ tools: [bash(T0 - 125_000)] }) }))[0]).toMatchObject({
            rank: RANK.anomaly,
            text: "Running the build · no output for 2m 5s",
        });
        expect(statusCandidates(input({ activity: act({ tools: [bash(T0 - 10_000)] }) }))[0].rank).toBe(RANK.now);
        // Never wrote anything: not news.
        expect(statusCandidates(input({ activity: act({ tools: [bash(undefined)] }) }))[0].rank).toBe(RANK.now);
    });
});

describe("what slow means in this pane", () => {
    it("20 s, or twice its typical wait once it has enough", () => {
        expect(slowRequestMs(undefined)).toBe(TIMING.slowRequestMs);
        expect(slowRequestMs([30_000, 30_000, 30_000, 30_000])).toBe(TIMING.slowRequestMs); // too few
        expect(slowRequestMs([2_000, 3_000, 4_000, 3_000, 5_000])).toBe(TIMING.slowRequestMs);
        expect(slowRequestMs([18_000, 20_000, 22_000, 19_000, 21_000])).toBe(40_000);
    });

    it("a pane whose model is always slow doesn't call each wait slow", () => {
        const waits = [18_000, 20_000, 22_000, 19_000, 21_000];
        const at = (age: number) => statusCandidates(input({ activity: act({ phase: "requesting", phaseSince: T0 - age, waits }) }))[0].rank;
        expect(at(30_000)).toBe(RANK.goal);
        expect(at(41_000)).toBe(RANK.anomaly);
    });
});
