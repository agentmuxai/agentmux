// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { BackgroundTaskView } from "@/app/store/rpc-api";
import { groupBackgroundTasks, isBackgroundTaskVisible, NO_BACKGROUND_TASKS } from "./swarm-background";

function task(overrides: Partial<BackgroundTaskView> = {}): BackgroundTaskView {
    return {
        id: "toolu_1",
        block_id: "block-a",
        label: "Background wait for agents",
        pid: null,
        started_at_ms: 1000,
        status: "running",
        last_seen_ms: 1000,
        ended_at_ms: null,
        owner_tool_use_id: null,
        ...overrides,
    };
}

describe("groupBackgroundTasks", () => {
    it("nests a task under the subagent that owns it, and keeps the agent's own at the top level", () => {
        const tasks = [
            task({ id: "own", owner_tool_use_id: null, started_at_ms: 1 }),
            task({ id: "sub", owner_tool_use_id: "toolu_agent", started_at_ms: 2 }),
        ];
        const g = groupBackgroundTasks(tasks, "block-a", new Set(["toolu_agent"]));
        expect(g.own.map((t) => t.id)).toEqual(["own"]);
        expect(g.bySubagent.get("toolu_agent")?.map((t) => t.id)).toEqual(["sub"]);
    });

    it("never hides a task whose owning subagent has no row: it falls back to the agent", () => {
        const g = groupBackgroundTasks([task({ owner_tool_use_id: "toolu_retired" })], "block-a", new Set(["toolu_other"]));
        expect(g.own).toHaveLength(1);
        expect(g.bySubagent.size).toBe(0);
    });

    it("only takes this agent's tasks, newest first", () => {
        const tasks = [
            task({ id: "old", started_at_ms: 1 }),
            task({ id: "elsewhere", block_id: "block-b", started_at_ms: 3 }),
            task({ id: "new", started_at_ms: 2 }),
        ];
        expect(groupBackgroundTasks(tasks, "block-a", new Set()).own.map((t) => t.id)).toEqual(["new", "old"]);
    });

    it("returns the shared empty value when there is nothing to show", () => {
        expect(groupBackgroundTasks([], "block-a", new Set())).toBe(NO_BACKGROUND_TASKS);
        expect(groupBackgroundTasks([task()], null, new Set())).toBe(NO_BACKGROUND_TASKS);
        expect(groupBackgroundTasks([task({ block_id: "block-b" })], "block-a", new Set())).toBe(NO_BACKGROUND_TASKS);
    });
});

describe("isBackgroundTaskVisible", () => {
    it("shows a running task however old", () => {
        expect(isBackgroundTaskVisible(task({ started_at_ms: 0 }), 10 * 3600_000)).toBe(true);
    });

    it("shows a finished task for its status's retention window, like the dock", () => {
        const done = task({ status: "done", ended_at_ms: 10_000 });
        expect(isBackgroundTaskVisible(done, 17_000)).toBe(true);
        expect(isBackgroundTaskVisible(done, 18_500)).toBe(false);
        const failed = task({ status: "error", ended_at_ms: 10_000 });
        expect(isBackgroundTaskVisible(failed, 24_000)).toBe(true);
        expect(isBackgroundTaskVisible(failed, 25_500)).toBe(false);
    });

    it("uses last_seen_ms when an ended row has no ended_at_ms", () => {
        const t = task({ status: "stopped", ended_at_ms: null, last_seen_ms: 5_000 });
        expect(isBackgroundTaskVisible(t, 7_000)).toBe(true);
        expect(isBackgroundTaskVisible(t, 9_000)).toBe(false);
    });
});
