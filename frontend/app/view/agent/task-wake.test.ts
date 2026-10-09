// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { createTaskWakeDetector } from "./task-wake";

const notification = (task_id: string, summary?: string) => ({ type: "system", subtype: "task_notification", task_id, tool_use_id: "toolu_1", status: "completed", summary });
const init = { type: "system", subtype: "init", session_id: "s" };
const result = { type: "result", subtype: "success" };
const userInput = { type: "user", message: { role: "user", content: [{ type: "text", text: "hi" }] } };
const toolResult = { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "toolu_2", content: "ok" }] } };

/** Feed lines in order, collecting the wake lines produced. */
function run(lines: unknown[]) {
    const detect = createTaskWakeDetector();
    return lines.map((l, i) => detect(l, 1_000 + i)).filter((n) => n != null);
}

describe("createTaskWakeDetector", () => {
    it("the order Claude Code was recorded emitting: result, task_notification, init → one wake line", () => {
        const done = 'Background command "npm test" completed (exit code 0)';
        expect(run([result, { type: "system", subtype: "task_updated" }, notification("b6ew", done), init])).toEqual([
            { type: "ambient_narration", id: "task-wake-b6ew", kind: "turn_trigger", text: `Woke up: ${done}`, timestamp: 1_003 },
        ]);
    });

    it("a task that finishes mid-pass marks the continuation pass the CLI starts for it", () => {
        expect(run([notification("t1"), toolResult, result, init]).map((n) => n.id)).toEqual(["task-wake-t1"]);
    });

    it("input before the next pass makes that pass the input's, not the task's", () => {
        expect(run([notification("t1"), result, userInput, init])).toEqual([]);
    });

    it("an init with no task notification before it, or a subagent's, adds nothing", () => {
        expect(run([init, result, init])).toEqual([]);
        expect(run([{ ...notification("t1"), parent_tool_use_id: "toolu_9" }, init])).toEqual([]);
    });

    it("one notification makes one line", () => {
        expect(run([notification("t1", "done"), init, result, init]).map((n) => n.text)).toEqual(["Woke up: done"]);
    });

    it("live and replay produce the same node (same id) for the same lines", () => {
        const lines = [result, notification("t7", "built"), init];
        expect(run(lines)[0].id).toBe(run(lines)[0].id);
    });
});
