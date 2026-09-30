// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A finished background task must leave the Activity Dock in EVERY pane, not
 * only one whose srv saw it finish live.
 *
 * Regression (2026-09-30): two docked "running" commands sat in AgentX's dock
 * for ~11.5 hours after both had finished. The dock learned that a background
 * launch ended from only two places — a `<task-notification>` USER message (the
 * CLI's stream never carries one) and srv's live registry (`applyRegistryOutcomes`).
 * A pane opened on a fresh instance replays the persisted stream into an empty
 * registry, so every background launch inside the replay window stayed
 * `running` forever, although the stream itself says it ended
 * (`system/task_notification`). The stream is durable and the registry is not;
 * the dock now reads the stream.
 */

import { describe, expect, it } from "vitest";
import { parseHistoryLines } from "../parseHistoryLines";
import type { ToolNode } from "../types";
import { registerPane, unregisterPane } from "@/app/store/agent-pane-registration";
import {
    applyStreamOutcomes,
    noteTaskFrame,
    registerTaskOutcomes,
    TaskOutcomeTracker,
    taskOutcomesFor,
    unregisterTaskOutcomes,
} from "./task-outcomes";
import { toolActivities } from "./tool-adapter";
import type { PinnedActivity } from "./types";

const started = (task_id: string, tool_use_id: string, extra: Record<string, unknown> = {}) => ({
    type: "system",
    subtype: "task_started",
    task_id,
    tool_use_id,
    task_type: "local_bash",
    ...extra,
});
const notified = (task_id: string, tool_use_id: string, status = "completed") => ({
    type: "system",
    subtype: "task_notification",
    task_id,
    tool_use_id,
    status,
});
const updated = (task_id: string, patch: Record<string, unknown>) => ({
    type: "system",
    subtype: "task_updated",
    task_id,
    patch,
});

describe("TaskOutcomeTracker", () => {
    it("records a completed notification against the launching tool_use_id", () => {
        const t = new TaskOutcomeTracker();
        expect(t.note(started("b1", "toolu_1"), 1000)).toBe(false);
        expect(t.note(notified("b1", "toolu_1"), 5000)).toBe(true);
        expect(t.outcomes.get("toolu_1")).toEqual({ status: "done", endedAt: 5000 });
    });

    it("maps failed to error and anything else (stopped, killed, unknown) to stopped", () => {
        const t = new TaskOutcomeTracker();
        t.note(notified("a", "toolu_a", "failed"), 1);
        t.note(notified("b", "toolu_b", "stopped"), 2);
        t.note(notified("c", "toolu_c", "something-new"), 3);
        expect(t.outcomes.get("toolu_a")?.status).toBe("error");
        expect(t.outcomes.get("toolu_b")?.status).toBe("stopped");
        expect(t.outcomes.get("toolu_c")?.status).toBe("stopped");
    });

    it("a task_updated end is resolved through the task_started that named its tool_use_id", () => {
        const t = new TaskOutcomeTracker();
        t.note(started("b1", "toolu_1"), 1000);
        expect(t.note(updated("b1", { status: "killed", end_time: 9 }), 7000)).toBe(true);
        expect(t.outcomes.get("toolu_1")).toEqual({ status: "stopped", endedAt: 7000 });
    });

    it("an end seen BEFORE its task_started (an older history page loads later) still lands", () => {
        const t = new TaskOutcomeTracker();
        t.note(updated("b1", { status: "completed" }), 7000);
        expect(t.outcomes.size).toBe(0);
        expect(t.note(started("b1", "toolu_1"), 1000)).toBe(true);
        expect(t.outcomes.get("toolu_1")).toEqual({ status: "done", endedAt: 7000 });
    });

    it("ignores frames that are not about a task ending, and non-terminal updates", () => {
        const t = new TaskOutcomeTracker();
        expect(t.note(updated("b1", { is_backgrounded: true }), 1)).toBe(false);
        expect(t.note(updated("b1", { status: "running" }), 1)).toBe(false);
        expect(t.note({ type: "assistant" }, 1)).toBe(false);
        expect(t.note({ type: "system", subtype: "init" }, 1)).toBe(false);
        expect(t.note(null as never, 1)).toBe(false);
        expect(t.outcomes.size).toBe(0);
    });

    it("the same end seen twice is not a change", () => {
        const t = new TaskOutcomeTracker();
        t.note(notified("b1", "toolu_1"), 5000);
        expect(t.note(notified("b1", "toolu_1"), 5000)).toBe(false);
    });

    it("an unstamped frame still ends the task (endedAt simply unknown)", () => {
        const t = new TaskOutcomeTracker();
        t.note(notified("b1", "toolu_1"), undefined);
        expect(t.outcomes.get("toolu_1")).toEqual({ status: "done", endedAt: undefined });
    });
});

describe("applyStreamOutcomes", () => {
    const row = (over: Partial<PinnedActivity>): PinnedActivity => ({
        id: "toolu_1",
        kind: "tool",
        title: "poll",
        status: "running",
        startedAt: 1000,
        canStop: false,
        ...over,
    });

    it("closes a running row the stream says ended", () => {
        const out = applyStreamOutcomes([row({})], new Map([["toolu_1", { status: "done" as const, endedAt: 5000 }]]));
        expect(out[0]).toMatchObject({ status: "done", endedAt: 5000 });
    });

    it("leaves other rows, and rows that already ended, alone", () => {
        const done = row({ id: "toolu_2", status: "done", endedAt: 2000 });
        const other = row({ id: "toolu_3" });
        const out = applyStreamOutcomes(
            [done, other],
            new Map([
                ["toolu_2", { status: "error" as const, endedAt: 9000 }],
                ["toolu_1", { status: "done" as const, endedAt: 5000 }],
            ]),
        );
        expect(out[0]).toBe(done);
        expect(out[1]).toBe(other);
    });

    it("returns the very same array when nothing changes (no needless recompute)", () => {
        const rows = [row({})];
        expect(applyStreamOutcomes(rows, new Map())).toBe(rows);
    });

    it("an unknown end time falls back to the row's own start, so retention can still expire it", () => {
        const out = applyStreamOutcomes([row({ startedAt: 1000 })], new Map([["toolu_1", { status: "done" as const }]]));
        expect(out[0].endedAt).toBe(1000);
    });
});

describe("taskOutcomesFor (per-pane store)", () => {
    it("fills from frames and keeps panes apart", () => {
        registerTaskOutcomes("pane-a");
        registerTaskOutcomes("pane-b");
        noteTaskFrame("pane-a", notified("b1", "toolu_1"), 5000);
        expect(taskOutcomesFor("pane-a")().get("toolu_1")?.status).toBe("done");
        expect(taskOutcomesFor("pane-b")().size).toBe(0);
        unregisterTaskOutcomes("pane-a");
        unregisterTaskOutcomes("pane-b");
    });

    it("hands out a NEW map only when something changed", () => {
        registerTaskOutcomes("pane-c");
        noteTaskFrame("pane-c", notified("b1", "toolu_1"), 5000);
        const before = taskOutcomesFor("pane-c")();
        noteTaskFrame("pane-c", { type: "assistant" }, 6000);
        noteTaskFrame("pane-c", notified("b1", "toolu_1"), 5000);
        expect(taskOutcomesFor("pane-c")()).toBe(before);
        noteTaskFrame("pane-c", notified("b2", "toolu_2"), 7000);
        expect(taskOutcomesFor("pane-c")()).not.toBe(before);
        unregisterTaskOutcomes("pane-c");
    });

    it("re-registering a pane starts it clean (its history is re-read)", () => {
        registerTaskOutcomes("pane-d");
        noteTaskFrame("pane-d", notified("b1", "toolu_1"), 5000);
        registerTaskOutcomes("pane-d");
        expect(taskOutcomesFor("pane-d")().size).toBe(0);
        unregisterTaskOutcomes("pane-d");
    });

    it("a closed pane keeps nothing, and a late frame for it (an in-flight history page) creates nothing", () => {
        registerTaskOutcomes("pane-e");
        noteTaskFrame("pane-e", notified("b1", "toolu_1"), 5000);
        unregisterTaskOutcomes("pane-e");
        expect(taskOutcomesFor("pane-e")().size).toBe(0);
        noteTaskFrame("pane-e", notified("b2", "toolu_2"), 6000);
        expect(taskOutcomesFor("pane-e")().size).toBe(0);
        // Still empty after "re-opening" proves the late frame left no entry behind.
        registerTaskOutcomes("pane-e");
        expect(taskOutcomesFor("pane-e")().size).toBe(0);
        unregisterTaskOutcomes("pane-e");
    });

    it("lives exactly as long as the pane-state slot: opened by registerPane, dropped by unregisterPane", () => {
        registerPane("pane-f", { agentId: "agentx" });
        noteTaskFrame("pane-f", notified("b1", "toolu_1"), 5000);
        expect(taskOutcomesFor("pane-f")().get("toolu_1")?.status).toBe("done");
        unregisterPane("pane-f");
        expect(taskOutcomesFor("pane-f")().size).toBe(0);
        noteTaskFrame("pane-f", notified("b2", "toolu_2"), 6000);
        expect(taskOutcomesFor("pane-f")().size).toBe(0);
    });
});

describe("the real failure: a finished background launch replayed into a fresh pane", () => {
    const line = (o: unknown) => JSON.stringify(o);
    const history = [
        line({
            type: "assistant",
            timestamp: "2026-09-30T09:52:24.134Z",
            message: {
                role: "assistant",
                content: [
                    {
                        type: "tool_use",
                        id: "toolu_poll",
                        name: "Bash",
                        input: { command: "for i in 1 2 3; do sleep 20; done", run_in_background: true },
                    },
                ],
            },
        }),
        line(started("b13o96q9j", "toolu_poll", { is_backgrounded: true })),
        line({
            type: "user",
            message: {
                role: "user",
                content: [
                    {
                        type: "tool_result",
                        tool_use_id: "toolu_poll",
                        content:
                            "Command running in background with ID: b13o96q9j. Output is being written to: /tmp/x.output",
                    },
                ],
            },
        }),
        line(notified("b13o96q9j", "toolu_poll")),
    ];

    it("shows running from the transcript alone (why the row stuck) and done once the stream's own end is applied", () => {
        const tracker = new TaskOutcomeTracker();
        // Replay stamps every line with its receive time; without one the
        // parser leaves a tool node un-timestamped and the dock skips it.
        const t0 = Date.parse("2026-09-30T09:52:24Z");
        const stamps = [t0, t0, t0 + 1_000, t0 + 360_000];
        const { nodes } = parseHistoryLines(history, "claude-stream-json", "AgentX", stamps, {
            onTaskFrame: (frame, at) => tracker.note(frame, at),
        });
        const bash = nodes.filter((n): n is ToolNode => n.type === "tool");
        expect(bash).toHaveLength(1);

        const rows = toolActivities(nodes, Date.parse("2026-09-30T21:30:00Z"));
        expect(rows).toHaveLength(1);
        expect(rows[0].status).toBe("running"); // before: nothing in the transcript ends it

        const closed = applyStreamOutcomes(rows, tracker.outcomes);
        expect(closed[0].status).toBe("done");
    });

    it("the history parser hands every system frame to its observer, stamped with the line's receive time", () => {
        const seen: Array<[string, number | undefined]> = [];
        parseHistoryLines(history, "claude-stream-json", "AgentX", [0, 111, 0, 222], {
            onTaskFrame: (frame, at) => seen.push([String(frame.subtype), at]),
        });
        expect(seen).toContainEqual(["task_started", 111]);
        expect(seen).toContainEqual(["task_notification", 222]);
    });
});
