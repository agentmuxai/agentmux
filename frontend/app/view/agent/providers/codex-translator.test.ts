// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import commandFixture from "../../../../test/fixtures/providers/codex/0.116.0/command.jsonl?raw";
import failureFixture from "../../../../test/fixtures/providers/codex/0.116.0/failure.jsonl?raw";
import fileChangeFixture from "../../../../test/fixtures/providers/codex/0.116.0/file-change.jsonl?raw";
import normalFixture from "../../../../test/fixtures/providers/codex/0.116.0/normal.jsonl?raw";
import type { StreamEvent } from "../types";
import { CodexTranslator } from "./codex-translator";

function translateFixture(raw: string): StreamEvent[] {
    const translator = new CodexTranslator();
    return raw
        .trim()
        .split(/\r?\n/)
        .flatMap((line) => translator.translate(JSON.parse(line)));
}

describe("CodexTranslator", () => {
    it("translates the pinned normal-turn fixture", () => {
        expect(translateFixture(normalFixture)).toEqual([
            { type: "text", content: "fixture-ok" },
            { type: "session_end", stats: { input_tokens: 11478, output_tokens: 26 } },
        ]);
    });

    it("opens, streams, and completes command_execution snapshots", () => {
        expect(translateFixture(commandFixture)).toEqual([
            { type: "text", content: "Running the requested shell command." },
            {
                type: "tool_call",
                tool: "Shell",
                id: "item_1",
                params: { command: "pwsh -Command Write-Output fixture-command-output" },
            },
            { type: "tool_chunk", id: "item_1", kind: "stdout", content: "fixture-command-output\r\n" },
            {
                type: "tool_result",
                tool: "Shell",
                id: "item_1",
                status: "success",
                result: { output: "fixture-command-output\r\n", status: "completed" },
                exitCode: 0,
            },
            { type: "text", content: "fixture-done" },
            { type: "session_end", stats: { input_tokens: 23315, output_tokens: 194 } },
        ]);
    });

    it("synthesizes a complete tool lifecycle when file_change has no started event", () => {
        const events = translateFixture(fileChangeFixture);
        expect(events[1]).toEqual({
            type: "tool_call",
            tool: "FileChange",
            id: "item_1",
            params: { changes: [{ path: "C:\\fixture-workspace\\fixture.txt", kind: "add" }] },
        });
        expect(events[2]).toEqual({
            type: "tool_result",
            tool: "FileChange",
            id: "item_1",
            status: "success",
            result: [{ path: "C:\\fixture-workspace\\fixture.txt", kind: "add" }],
        });
    });

    it("deduplicates the paired error and turn.failed frames", () => {
        expect(translateFixture(failureFixture)).toEqual([
            { type: "error_result", code: 400, message: "Fixture request failed" },
            { type: "session_end", stats: {} },
        ]);
    });

    it("emits only suffixes from repeated item.updated snapshots", () => {
        const translator = new CodexTranslator();
        translator.translate({ type: "turn.started" });
        const item = {
            id: "cmd",
            type: "command_execution",
            command: "echo hi",
            status: "in_progress",
            exit_code: null,
        };
        expect(translator.translate({ type: "item.started", item: { ...item, aggregated_output: "" } })).toHaveLength(
            1
        );
        expect(translator.translate({ type: "item.updated", item: { ...item, aggregated_output: "one" } })).toEqual([
            { type: "tool_chunk", id: "cmd", kind: "stdout", content: "one" },
        ]);
        expect(translator.translate({ type: "item.updated", item: { ...item, aggregated_output: "one two" } })).toEqual(
            [{ type: "tool_chunk", id: "cmd", kind: "stdout", content: " two" }]
        );
    });

    it("uses first-terminal-wins semantics", () => {
        const translator = new CodexTranslator();
        expect(translator.translate({ type: "turn.completed", usage: { input_tokens: 1 } })).toEqual([
            { type: "session_end", stats: { input_tokens: 1 } },
        ]);
        expect(translator.translate({ type: "turn.failed", error: { message: "late failure" } })).toEqual([]);
    });

    it("fails tools left open by turn.failed", () => {
        const translator = new CodexTranslator();
        translator.translate({ type: "turn.started" });
        translator.translate({
            type: "item.started",
            item: {
                id: "cmd",
                type: "command_execution",
                command: "long-running",
                aggregated_output: "",
                status: "in_progress",
            },
        });
        expect(translator.translate({ type: "turn.failed", error: { message: "provider stopped" } })).toEqual([
            { type: "error_result", code: 0, message: "provider stopped" },
            {
                type: "tool_result",
                tool: "Shell",
                id: "cmd",
                status: "failed",
                result: { error: "provider stopped" },
            },
            { type: "session_end", stats: {} },
        ]);
    });

    it("ignores unknown events and item types without throwing", () => {
        const translator = new CodexTranslator();
        expect(translator.translate({ type: "future.event", value: 1 })).toEqual([]);
        expect(
            translator.translate({
                type: "item.completed",
                item: { id: "future", type: "future_item", value: 1 },
            })
        ).toEqual([]);
    });

    it("keeps legacy function_call correlation compatible", () => {
        const translator = new CodexTranslator();
        expect(
            translator.translate({
                type: "item.completed",
                item: {
                    id: "legacy",
                    type: "function_call",
                    name: "lookup",
                    call_id: "call-1",
                    arguments: '{"q":"x"}',
                },
            })
        ).toEqual([{ type: "tool_call", tool: "lookup", id: "call-1", params: { q: "x" } }]);
        expect(
            translator.translate({
                type: "item.completed",
                item: { id: "legacy-result", type: "function_call_output", call_id: "call-1", output: "ok" },
            })
        ).toEqual([
            {
                type: "tool_result",
                tool: "lookup",
                id: "call-1",
                status: "success",
                result: { output: "ok" },
            },
        ]);
    });
});
