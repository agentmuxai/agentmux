// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

import { keepUnchanged, taskForOpenRequest } from "./kanban-model";
import type { KanbanBoard } from "./kanban-types";

const board = (titles: string[], state = "idle"): KanbanBoard =>
    ({
        tasks: titles.map((title, i) => ({ id: `w${i}`, title, card: { num: i } })),
        agents: [{ name: "A", block_id: "b1", state }],
        stamp: 1,
        now: 1,
    }) as unknown as KanbanBoard;

describe("a refresh keeps what didn't change", () => {
    it("reuses an unchanged task or agent object and takes a changed one", () => {
        const prev = board(["one", "two"]);
        const next = keepUnchanged(prev, board(["one", "two, renamed"], "working"));
        expect(next.tasks[0]).toBe(prev.tasks[0]);
        expect(next.tasks[1]).not.toBe(prev.tasks[1]);
        expect(next.tasks[1].title).toBe("two, renamed");
        expect(next.agents[0]).not.toBe(prev.agents[0]);
        expect(keepUnchanged(next, board(["one", "two, renamed"], "working")).agents[0]).toBe(next.agents[0]);
    });

    it("takes the new board whole the first time", () => {
        const b = board(["one"]);
        expect(keepUnchanged(null, b)).toBe(b);
    });
});


describe("a request to open a task", () => {
    it("names it by id or by the number the operator sees", () => {
        const { tasks } = board(["one", "two"]);
        expect(taskForOpenRequest(tasks, "w1")?.title).toBe("two");
        expect(taskForOpenRequest(tasks, "1")?.title).toBe("two");
        expect(taskForOpenRequest(tasks, "7")).toBeUndefined();
    });
});
