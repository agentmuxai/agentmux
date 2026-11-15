// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { resolveAttachedTaskObservation } from "./useBackgroundTaskRegistry";
import type { BackgroundTaskView } from "@/app/store/rpc-api";

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
        owner_tool_use_id: null,
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

    it("does not count a row with no pid that nothing has confirmed for hours", () => {
        const now = 100 * 60 * 60 * 1000;
        const orphan = task({ id: "bg-orphan", pid: null, started_at_ms: now - 17 * 60 * 60 * 1000, last_seen_ms: now - 17 * 60 * 60 * 1000 });
        expect(resolveAttachedTaskObservation([orphan], now)).toBeNull();
        // The live row alone decides `since`; the orphan's older start doesn't pull it back.
        const live = task({ id: "bg-live", pid: 4242, started_at_ms: now - 1000, last_seen_ms: now - 1000 });
        expect(resolveAttachedTaskObservation([orphan, live], now)).toBe(now - 1000);
    });

    it("still counts a row with no pid seen recently, and a row with a pid however old", () => {
        const now = 100 * 60 * 60 * 1000;
        expect(resolveAttachedTaskObservation([task({ pid: null, started_at_ms: now - 5000, last_seen_ms: now - 5000 })], now)).toBe(now - 5000);
        expect(resolveAttachedTaskObservation([task({ pid: 4242, started_at_ms: 1, last_seen_ms: 1 })], now)).toBe(1);
    });

    it("ignores non-running tasks when computing the earliest start", () => {
        const tasks = [
            task({ id: "bg-1", status: "done", started_at_ms: 100 }),
            task({ id: "bg-2", status: "running", started_at_ms: 5000 }),
        ];
        expect(resolveAttachedTaskObservation(tasks)).toBe(5000);
    });
});
