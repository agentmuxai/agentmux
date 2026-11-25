// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import type { KanbanTask } from "@/app/view/kanban/kanban-types";
import { followHeldTasks, heldTaskOf, publishHeldTasks, registerTaskFollower } from "./held-tasks";

const task = (num: number, pane: string | null, state: string, at: number): KanbanTask =>
    ({ title: `Task ${num}`, state, updated_at: at, card: { num }, holder: pane ? { block_id: pane, name: "x", state: "working" } : null }) as unknown as KanbanTask;

describe("the task each agent pane holds", () => {
    it("is nothing until the boards are read", () => {
        expect(heldTaskOf("b1")).toBeUndefined();
    });

    it("is the newest claimed, asking or in-review task of the pane", () => {
        publishHeldTasks([task(1, "b1", "claimed", 10), task(2, "b1", "review", 30), task(3, "b2", "blocked", 5)]);
        expect(heldTaskOf("b1")).toEqual({ num: 2, title: "Task 2" });
        expect(heldTaskOf("b2")).toEqual({ num: 3, title: "Task 3" });
    });

    it("leaves out a task that is over, waiting or held by nobody, and drops a pane that no longer holds one", () => {
        publishHeldTasks([task(4, "b1", "done", 40), task(5, "b1", "open", 50), task(6, null, "claimed", 60)]);
        expect(heldTaskOf("b1")).toBeUndefined();
        expect(heldTaskOf("b2")).toBeUndefined();
    });

    it("asks the registered follower to read the boards, and does nothing before one is registered", () => {
        expect(() => followHeldTasks()).not.toThrow();
        const follow = vi.fn();
        registerTaskFollower(follow);
        followHeldTasks();
        expect(follow).toHaveBeenCalledTimes(1);
    });
});
