// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { resolveAttachedTaskObservation } from "./useBackgroundTaskRegistry";

// Phase C of docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md.

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

describe("resolveAttachedTaskObservation", () => {
    it("returns null for an empty list", () => {
        expect(resolveAttachedTaskObservation([])).toBeNull();
    });

    it("returns null when nothing is running", () => {
        const tasks = [task({ status: "done" }), task({ id: "bg-2", status: "error" }), task({ id: "bg-3", status: "stopped" })];
        expect(resolveAttachedTaskObservation(tasks)).toBeNull();
    });

    it("returns the started_at_ms of a single running task", () => {
        expect(resolveAttachedTaskObservation([task({ started_at_ms: 5000 })])).toBe(5000);
    });

    it("returns the EARLIEST started_at_ms across multiple running tasks", () => {
        const tasks = [
            task({ id: "bg-1", started_at_ms: 9000 }),
            task({ id: "bg-2", started_at_ms: 2000 }),
            task({ id: "bg-3", started_at_ms: 5000 }),
        ];
        expect(resolveAttachedTaskObservation(tasks)).toBe(2000);
    });

    it("ignores non-running tasks when computing the earliest start", () => {
        const tasks = [
            task({ id: "bg-1", status: "done", started_at_ms: 100 }),
            task({ id: "bg-2", status: "running", started_at_ms: 5000 }),
        ];
        expect(resolveAttachedTaskObservation(tasks)).toBe(5000);
    });
});
