// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.3: the reducer keeps
// the facts the live status reads; the presenter decides what to say.

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { initialState, type AgentPaneState } from "./types";

const at = (s: AgentPaneState, command: Parameters<typeof update>[1], ms: number) => update(s, command, ms).state;
/** A pane in a turn whose first call has started (live tokens exist). */
const inCall = () => at(initialState("a"), { type: "TokensIn", input: 1_000 }, 1_000);

describe("activity: the model's phase", () => {
    it("a call starts responding; its stream says thinking, writing or composing", () => {
        let s = inCall();
        expect(s.activity).toMatchObject({ phase: "responding", phaseSince: 1_000 });
        s = at(s, { type: "OutputStreamed", chars: 40, kind: "thinking" }, 1_200);
        expect(s.activity).toMatchObject({ phase: "thinking", phaseSince: 1_200 });
        s = at(s, { type: "OutputStreamed", chars: 40, kind: "thinking" }, 1_900);
        expect(s.activity.phaseSince).toBe(1_200); // the same phase keeps its start
        s = at(s, { type: "OutputStreamed", chars: 10, kind: "tool_input" }, 2_500);
        expect(s.activity.phase).toBe("composing");
        s = at(s, { type: "OutputStreamed", chars: 10, kind: "text" }, 3_000);
        expect(s.activity.phase).toBe("writing");
    });

    it("tool results going back: requesting, until the next call", () => {
        let s = at(inCall(), { type: "RequestStarted" }, 5_000);
        expect(s.activity).toMatchObject({ phase: "requesting", phaseSince: 5_000 });
        s = at(s, { type: "TokensIn", input: 1_500 }, 9_000);
        expect(s.activity).toMatchObject({ phase: "responding", phaseSince: 9_000 });
    });
});

describe("activity: tools in flight", () => {
    it("a call joins the list with its words, and its end removes exactly it", () => {
        let s = inCall();
        s = at(s, { type: "ToolStart", name: "Bash", id: "t1", params: { description: "Run the tests" } }, 2_000);
        s = at(s, { type: "ToolStart", name: "Read", id: "t2", params: { file_path: "/x/a.ts" } }, 2_100);
        expect(s.activity.tools.map((t) => [t.id, t.activity.label, t.startedAt])).toEqual([
            ["t1", "Running the tests", 2_000],
            ["t2", "Reading a.ts", 2_100],
        ]);
        s = at(s, { type: "ToolEnd", id: "t2" }, 2_500);
        expect(s.activity.tools.map((t) => t.id)).toEqual(["t1"]);
        s = at(s, { type: "ToolEnd" }, 2_600); // no id: the oldest
        expect(s.activity.tools).toEqual([]);
    });

    it("a todo list sets the plan, which outlives the pass", () => {
        let s = inCall();
        const todos = [
            { content: "Read the code", activeForm: "Reading the code", status: "completed" },
            { content: "Write the spec", activeForm: "Writing the spec", status: "in_progress" },
            { content: "Open the PR", activeForm: "Opening the PR", status: "pending" },
        ];
        s = at(s, { type: "ToolStart", name: "TodoWrite", id: "p", params: { todos } }, 3_000);
        expect(s.activity.plan).toEqual({ activeForm: "Writing the spec", index: 2, total: 3 });
        expect(s.activity.planAt).toBe(3_000);
        s = at(s, { type: "TurnEnd", stats: null }, 4_000);
        expect(s.activity).toMatchObject({ phase: null, tools: [], plan: { index: 2 } });
    });

    it("a reset clears it all", () => {
        let s = at(inCall(), { type: "ToolStart", name: "Bash", id: "t", params: {} }, 2_000);
        s = at(s, { type: "TurnReset" }, 3_000);
        expect(s.activity).toMatchObject({ phase: null, tools: [], plan: null });
    });
});

describe("activity: subagent steps, progress, the thinking headline", () => {
    it("a subagent's call keeps its parent; progress lands on its call; a new call drops the headline", () => {
        let s = inCall();
        s = at(s, { type: "ToolStart", name: "Agent", id: "a1", params: { description: "map" } }, 2_000);
        s = at(s, { type: "ToolStart", name: "Read", id: "s1", params: { file_path: "a.ts" }, parentId: "a1" }, 2_100);
        expect(s.activity.tools[1]).toMatchObject({ id: "s1", parentId: "a1" });
        s = at(s, { type: "ToolProgress", id: "a1", text: "41/123" }, 2_200);
        expect(s.activity.tools[0].progress).toBe("41/123");
        s = at(s, { type: "ThinkingHeadline", text: "Weighing it" }, 2_300);
        expect(s.activity.thinkingHeadline).toBe("Weighing it");
        s = at(s, { type: "TokensIn", input: 2_000 }, 3_000);
        expect(s.activity.thinkingHeadline).toBeUndefined();
    });
});

describe("activity: this pane's waits for the model", () => {
    it("records how long each request waited before the answer began", () => {
        let s = at(inCall(), { type: "RequestStarted" }, 5_000);
        s = at(s, { type: "TokensIn", input: 2_000 }, 8_500);
        expect(s.activity.waits).toEqual([3_500]);
        s = at(s, { type: "TokensIn", input: 2_100 }, 9_000); // no request was waiting
        expect(s.activity.waits).toEqual([3_500]);
    });
});

describe("activity: one call reported twice (#4510)", () => {
    it("the placeholder and the full call are one entry, keeping its start; its end clears it", () => {
        let s = inCall();
        s = at(s, { type: "ToolStart", name: "Bash", id: "t1", params: {} }, 2_000); // streaming placeholder
        s = at(s, { type: "ToolProgress", id: "t1", text: "3/9" }, 2_100);
        s = at(s, { type: "ToolStart", name: "Bash", id: "t1", params: { description: "Run the tests" } }, 2_500); // the full call
        expect(s.activity.tools).toHaveLength(1);
        expect(s.activity.tools[0]).toMatchObject({ activity: { label: "Running the tests" }, startedAt: 2_000, progress: "3/9" });
        s = at(s, { type: "ToolEnd", id: "t1" }, 9_000);
        expect(s.activity.tools).toEqual([]);
    });

    it("a todo list's empty placeholder doesn't wipe the plan", () => {
        let s = inCall();
        const todos = [{ content: "Do it", activeForm: "Doing it", status: "in_progress" }];
        s = at(s, { type: "ToolStart", name: "TodoWrite", id: "p1", params: { todos } }, 2_000);
        s = at(s, { type: "ToolEnd", id: "p1" }, 2_100);
        s = at(s, { type: "ToolStart", name: "TodoWrite", id: "p2", params: {} }, 3_000); // the next one's placeholder
        expect(s.activity).toMatchObject({ plan: { activeForm: "Doing it" }, planAt: 2_000 });
    });
});
