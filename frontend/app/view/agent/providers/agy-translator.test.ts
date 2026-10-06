// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import toolsFixture from "../../../../test/fixtures/providers/agy/1.1.11/tools.jsonl?raw";
import resumeFixture from "../../../../test/fixtures/providers/agy/1.1.11/resume.jsonl?raw";
import emptyPromptFixture from "../../../../test/fixtures/providers/agy/1.1.11/empty-prompt.jsonl?raw";
import { AgyTranslator } from "./agy-translator";
import { createTranslator } from "./translator-factory";
import type { StreamEvent } from "../types";

// Real `agy` 1.1.11 output (SPEC_ANTIGRAVITY_HARNESS_REAL_CLI_2026_10_06.md),
// with the local user name and temp folder replaced.
function run(t: AgyTranslator, fixture: string): StreamEvent[] {
    return fixture
        .trim()
        .split("\n")
        .flatMap((line) => t.translate(JSON.parse(line)));
}

const CONV = "45628811-702f-4910-8096-9c5441de80f3";

const text = (events: StreamEvent[]) =>
    events
        .filter((e): e is Extract<StreamEvent, { type: "text" }> => e.type === "text")
        .map((e) => e.content)
        .join("");

const toolStep = (index: number, state: string, toolInfo: Record<string, unknown>, conversation = "c") => ({
    event: "step_update",
    step_update: { conversation_id: conversation, step_index: index, state, step_type: "tool", tool_name: "run_command", tool_info: toolInfo },
});

describe("AgyTranslator", () => {
    it("is what the factory builds for agy-stream-json", () => {
        expect(createTranslator("agy-stream-json")).toBeInstanceOf(AgyTranslator);
    });

    it("turns a run with two tools into calls, results, the reply and a turn end", () => {
        const events = run(new AgyTranslator(), toolsFixture);
        expect(events.filter((e) => e.type === "tool_call")).toEqual([
            {
                type: "tool_call",
                tool: "view_file",
                id: `agy-${CONV}-2`,
                params: { AbsolutePath: "C:\\Users\\user\\AppData\\Local\\Temp\\agy-fixture\\notes.txt" },
            },
            { type: "tool_call", tool: "run_command", id: `agy-${CONV}-3`, params: { CommandLine: "echo hello" } },
        ]);
        expect(events.filter((e) => e.type === "tool_result")).toEqual([
            {
                type: "tool_result",
                tool: "view_file",
                id: `agy-${CONV}-2`,
                status: "success",
                result: { output: "4 lines, 17 bytes" },
            },
            {
                type: "tool_result",
                tool: "run_command",
                id: `agy-${CONV}-3`,
                status: "success",
                result: { output: "hello\r\n" },
            },
        ]);
        expect(text(events)).toBe(
            "The second line of [notes.txt](file:///C:/Users/user/AppData/Local/Temp/agy-fixture/notes.txt) is **beta**.\n\nThe `echo hello` command also ran successfully, outputting `hello`.\n",
        );
        expect(events.at(-1)).toEqual({
            type: "session_end",
            stats: { input_tokens: 14610, output_tokens: 375, duration_ms: 13155, num_turns: 1 },
        });
    });

    it("emits each tool call once, though agy reports it twice", () => {
        const events = run(new AgyTranslator(), toolsFixture);
        expect(events.filter((e) => e.type === "tool_call")).toHaveLength(2);
    });

    // readToolResult re-reads one stored line with a fresh translator, so a
    // tool's id must come from that line alone. Step indexes keep counting
    // across a resumed conversation (5-7 in resume.jsonl, after 0-4), so the
    // conversation id plus the step index is unique.
    it("gives a tool the same id from its own line as from the whole run", () => {
        const doneLine = toolsFixture
            .trim()
            .split("\n")
            .find((l) => l.includes('"tool_name":"run_command"') && l.includes('"state":"DONE"'))!;
        const alone = new AgyTranslator().translate(JSON.parse(doneLine));
        expect(alone.find((e) => e.type === "tool_result")).toMatchObject({
            id: `agy-${CONV}-3`,
            status: "success",
            result: { output: "hello\r\n" },
        });
    });

    it("keeps the same step of different conversations apart", () => {
        const t = new AgyTranslator();
        const [a] = t.translate(toolStep(2, "ACTIVE", {}, "first"));
        const [b] = t.translate(toolStep(2, "ACTIVE", {}, "second"));
        expect(a).toMatchObject({ type: "tool_call", id: "agy-first-2" });
        expect(b).toMatchObject({ type: "tool_call", id: "agy-second-2" });
    });

    it("reports a tool seen only when it finished as both a call and its result", () => {
        const t = new AgyTranslator();
        const events = t.translate(toolStep(5, "DONE", { parameters: { CommandLine: "ls" }, output: "a.txt" }));
        expect(events.map((e) => e.type)).toEqual(["tool_call", "tool_result"]);
    });

    it("marks a tool that did not finish cleanly as failed", () => {
        const t = new AgyTranslator();
        const [, result] = t.translate(toolStep(1, "ERROR", { error: "denied" }));
        expect(result).toMatchObject({ type: "tool_result", status: "failed", result: { output: "denied" } });
    });

    it("shows nothing for the steps that carry no content", () => {
        const t = new AgyTranslator();
        expect(t.translate({ event: "init", conversation_id: "c", init: { tools: [] } })).toEqual([]);
        for (const step_type of ["user_input", "system_message", "checkpoint", "agent_response", "something_new"]) {
            expect(
                t.translate({ event: "step_update", step_update: { step_index: 0, state: "DONE", step_type } }),
            ).toEqual([]);
        }
    });

    it("a resumed turn translates like any other", () => {
        const events = run(new AgyTranslator(), resumeFixture);
        expect(text(events)).toBe("beta\n");
        expect(events.at(-1)?.type).toBe("session_end");
    });

    it("an ERROR result shows an error before ending the turn", () => {
        expect(run(new AgyTranslator(), emptyPromptFixture)).toEqual([
            { type: "text", content: "**Error:** antigravity turn ERROR" },
            { type: "session_end", stats: {} },
        ]);
    });

    it("renders the backend's user record on replay only", () => {
        const record = { type: "user", message: { content: "hello" } };
        expect(new AgyTranslator().translate(record)).toEqual([]);
        expect(new AgyTranslator({ replay: true }).translate(record)).toEqual([
            { type: "user_message", message: "hello" },
        ]);
    });
});
