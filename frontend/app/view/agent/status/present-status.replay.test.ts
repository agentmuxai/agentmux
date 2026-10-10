// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Replay tests for the live status (SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md
// §7): timed pane commands run through the real reducer, and the presenter is
// ticked the way the row ticks it, so what is checked is the line sequence a
// user would see. The scenarios are timings only, shaped like real turns; they
// carry no recorded content.
//
// The checks are the rules the row promises, not a snapshot of its text:
// - a line stays at least DWELL once fully typed out, unless something ranked
//   higher (or needs-you / held / an anomaly) replaces it;
// - the row doesn't flash back to the goal between two calls.

import { describe, expect, it } from "vitest";
import { update } from "@/app/store/agent-pane-state/reducer";
import { initialState, type AgentPaneCommand, type AgentPaneState } from "@/app/store/agent-pane-state/types";
import { presentStatus, RANK, TIMING, type StatusMemory } from "./present-status";

type Event = [ms: number, command: AgentPaneCommand];

interface Segment {
    text: string;
    key: string;
    rank: number;
    from: number;
    to: number;
}

interface Scenario {
    events: Event[];
    endMs: number;
    /** Windows in which a question or approval waits on the user. */
    needsYou?: Array<[from: number, to: number, text: string]>;
}

const TICK_MS = 100;
const GOAL = "Fix the login redirect loop";

/** Play a scenario; the row's view of it, one segment per line shown. */
function replay(s: Scenario): Segment[] {
    let state: AgentPaneState = update(initialState("a"), { type: "TokensIn", input: 1_000 }, 0).state;
    const events = [...s.events].sort((a, b) => a[0] - b[0]);
    let next = 0;
    let memory: StatusMemory | null = null;
    const segments: Segment[] = [];
    for (let now = 0; now <= s.endMs; now += TICK_MS) {
        while (next < events.length && events[next][0] <= now) {
            state = update(state, events[next][1], events[next][0]).state;
            next++;
        }
        const ask = s.needsYou?.find(([from, to]) => now >= from && now < to);
        const r = presentStatus(
            {
                nowMs: now,
                needsYou: ask ? ask[2] : null,
                held: null,
                leadIn: null,
                activity: state.activity,
                turnStartedAt: 0,
                goal: GOAL,
                phrase: "Working",
            },
            memory,
        );
        memory = r.memory;
        const last = segments.at(-1);
        if (last && last.key === r.line.key) {
            last.to = now + TICK_MS;
            last.text = r.line.text;
        } else {
            segments.push({ text: r.line.text, key: r.line.key, rank: r.line.rank, from: now, to: now + TICK_MS });
        }
    }
    return segments;
}

/** Every new line types out, so it is fully on screen this long after it came. */
const readyAt = (seg: Segment) => seg.from + seg.text.length * TIMING.revealCharMs;

/** Every rule the row promises, as a list of what broke (empty: all held). */
function violations(segments: Segment[]): string[] {
    const out: string[] = [];
    for (let i = 1; i < segments.length; i++) {
        const prev = segments[i - 1];
        const cur = segments[i];
        const preempts = cur.rank <= RANK.anomaly || cur.rank < prev.rank;
        // A same-or-lower rank waits out the dwell from the end of the type-out.
        if (!preempts && cur.from < readyAt(prev) + TIMING.dwellMs - TICK_MS) {
            out.push(`"${prev.text}" replaced by "${cur.text}" ${readyAt(prev) + TIMING.dwellMs - cur.from} ms early`);
        }
        // No flash of a calmer line between two calls.
        const after = segments[i + 1];
        if (prev.rank === RANK.now && cur.rank > RANK.now && after?.rank === RANK.now && cur.to - cur.from < 1_000) {
            out.push(`"${cur.text}" flashed for ${cur.to - cur.from} ms between two calls`);
        }
    }
    return out;
}

/** A call: its start, its end, and the request that follows its result. */
function call(id: string, name: string, params: Record<string, unknown>, start: number, end: number, parentId?: string): Event[] {
    const ev: Event[] = [
        [start, { type: "ToolStart", name, id, params, parentId }],
        [end, { type: "ToolEnd", id }],
    ];
    if (!parentId) ev.push([end, { type: "RequestStarted" }]);
    return ev;
}
const answer = (at: number): Event => [at, { type: "TokensIn", input: 1_000 }];
const stream = (at: number, kind: "text" | "thinking" | "tool_input"): Event => [at, { type: "OutputStreamed", chars: 40, kind }];

describe("replay: the row's promises hold across whole turns", () => {
    it("a burst of quick reads, then a long test run, then thinking and the reply", () => {
        const events: Event[] = [
            stream(200, "thinking"),
            stream(2_800, "tool_input"),
            ...call("r1", "Read", { file_path: "/src/auth/session.ts" }, 3_000, 3_400),
            answer(3_700),
            ...call("r2", "Read", { file_path: "/src/auth/cookie.ts" }, 3_800, 4_200),
            answer(4_400),
            ...call("r3", "Read", { file_path: "/src/auth/redirect.ts" }, 4_500, 4_900),
            answer(5_100),
            ...call("g1", "Grep", { pattern: "redirect_uri" }, 5_200, 5_900),
            answer(6_200),
            stream(6_300, "tool_input"),
            ...call("b1", "Bash", { description: "Run the auth tests" }, 6_800, 40_000),
            ...Array.from({ length: 15 }, (_, i): Event => [8_000 + i * 2_000, { type: "ToolOutput", id: "b1", at: 8_000 + i * 2_000 }]),
            answer(41_000),
            stream(41_100, "thinking"),
            [46_000, { type: "ThinkingHeadline", text: "Where the redirect is issued" }],
            stream(49_000, "text"),
        ];
        const segments = replay({ events, endMs: 56_000 });
        expect(violations(segments)).toEqual([]);
        // The quick reads never reach the screen; the test run and the thinking do.
        const texts = segments.map((s) => s.text);
        expect(texts.some((t) => t.startsWith("Reading"))).toBe(false);
        expect(texts.some((t) => t.startsWith("Running the auth tests"))).toBe(true);
        expect(texts.some((t) => t.startsWith("Thinking"))).toBe(true);
    });

    it("a steady loop of short calls doesn't flash the goal between them", () => {
        // Calls of 1.8–2.6 s with 0.6–1.4 s between them: each shows, and the
        // gap plus the next call's promote threshold outlasts HOLD.
        const events: Event[] = [];
        let t = 1_000;
        for (let i = 0; i < 20; i++) {
            const length = 1_800 + ((i * 370) % 800);
            const gap = 600 + ((i * 530) % 800);
            events.push(...call(`c${i}`, i % 3 === 0 ? "Bash" : "Edit", i % 3 === 0 ? { description: "Build the frontend" } : { file_path: `/src/f${i}.ts` }, t, t + length));
            events.push(answer(t + length + gap / 2), stream(t + length + gap / 2 + 50, "tool_input"));
            t += length + gap;
        }
        const segments = replay({ events, endMs: t + 4_000 });
        expect(violations(segments)).toEqual([]);
        // Between the first call showing and the last call ending, no goal.
        const firstNow = segments.findIndex((s) => s.rank === RANK.now);
        const lastNow = segments.map((s) => s.rank).lastIndexOf(RANK.now);
        expect(segments.slice(firstNow, lastNow).filter((s) => s.rank > RANK.now)).toEqual([]);
    });

    it("a subagent's steps swap in under its line without typing out again", () => {
        const events: Event[] = [
            ...call("ag", "Task", { description: "map the auth flow", subagent_type: "Explore" }, 500, 30_000),
            ...Array.from({ length: 12 }, (_, i) =>
                call(`s${i}`, i % 2 ? "Grep" : "Read", i % 2 ? { pattern: `term${i}` } : { file_path: `/src/m${i}.ts` }, 2_000 + i * 2_200, 3_200 + i * 2_200, "ag"),
            ).flat(),
        ];
        const segments = replay({ events, endMs: 33_000 });
        expect(violations(segments)).toEqual([]);
        const steps = segments.filter((s) => s.rank === RANK.now);
        // One rank-4 line: the subagent's. Its steps change the text, not the key.
        expect(new Set(steps.map((s) => s.key)).size).toBe(1);
    });

    it("an approval mid-run takes the row at once, and the run comes back after it", () => {
        const events: Event[] = [...call("b1", "Bash", { description: "Run the migration" }, 500, 20_000)];
        const segments = replay({ events, endMs: 22_000, needsYou: [[8_000, 12_000, "Waiting for your approval: Bash"]] });
        expect(violations(segments)).toEqual([]);
        const ask = segments.find((s) => s.rank === RANK.needsYou);
        expect(ask?.from).toBe(8_000);
        expect(segments[segments.indexOf(ask!) + 1].text.startsWith("Running the migration")).toBe(true);
    });
});
