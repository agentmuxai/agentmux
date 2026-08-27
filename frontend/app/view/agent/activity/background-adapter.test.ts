// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { backgroundTaskActivities, backgroundTaskToActivity } from "./background-adapter";

function task(overrides: Partial<BackgroundTaskView> = {}): BackgroundTaskView {
    return {
        id: "bg-1",
        block_id: "block-1",
        label: "task dev",
        pid: 4242,
        started_at_ms: 1000,
        status: "running",
        last_seen_ms: 1000,
        ended_at_ms: null,
        ...overrides,
    };
}

describe("backgroundTaskToActivity", () => {
    it("maps a running registry row to a running, non-stoppable activity", () => {
        const a = backgroundTaskToActivity(task({ id: "bg-1", label: "task dev", started_at_ms: 5000 }));
        expect(a.id).toBe("bg-1");
        expect(a.kind).toBe("tool");
        expect(a.title).toBe("task dev");
        expect(a.status).toBe("running");
        expect(a.startedAt).toBe(5000);
        expect(a.endedAt).toBeUndefined();
        expect(a.canStop).toBe(false);
        expect(a.tool).toBeUndefined();
    });

    it("carries a terminal status and endedAt straight through", () => {
        const a = backgroundTaskToActivity(task({ status: "error", ended_at_ms: 9000 }));
        expect(a.status).toBe("error");
        expect(a.endedAt).toBe(9000);
    });

    it("maps a null ended_at_ms to undefined, not null", () => {
        const a = backgroundTaskToActivity(task({ status: "done", ended_at_ms: null }));
        expect(a.endedAt).toBeUndefined();
    });
});

describe("backgroundTaskActivities", () => {
    it("returns every registry row when nothing is already known", () => {
        const tasks = [task({ id: "bg-1" }), task({ id: "bg-2" })];
        const out = backgroundTaskActivities(tasks, new Set());
        expect(out.map((a) => a.id)).toEqual(["bg-1", "bg-2"]);
    });

    it("filters out a row whose id already appears among the transcript-derived activities", () => {
        // BackgroundTaskView.id mirrors the originating tool_use_id
        // (background_tasks.rs's own doc comment) — a task the transcript
        // still has a record of (no restart) already has a PinnedActivity
        // from tool-adapter.ts's toolActivities under this exact id.
        const tasks = [task({ id: "bg-1" }), task({ id: "bg-2" })];
        const out = backgroundTaskActivities(tasks, new Set(["bg-1"]));
        expect(out.map((a) => a.id)).toEqual(["bg-2"]);
    });

    it("returns nothing when every row is already known", () => {
        const tasks = [task({ id: "bg-1" }), task({ id: "bg-2" })];
        const out = backgroundTaskActivities(tasks, new Set(["bg-1", "bg-2"]));
        expect(out).toEqual([]);
    });

    it("returns nothing for an empty task list", () => {
        expect(backgroundTaskActivities([], new Set())).toEqual([]);
    });
});
