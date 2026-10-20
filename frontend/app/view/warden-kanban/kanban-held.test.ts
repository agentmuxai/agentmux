// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { heldByBlock } from "./kanban-held";
import type { KanbanBoard } from "./kanban-types";

const task = (id: string, state: string, by: string, uid: string, updated_at: number, holder?: unknown) => ({
    id,
    title: `task ${id}`,
    state,
    claimed_by: by,
    claimed_by_uid: uid,
    updated_at,
    holder: holder ?? null,
    card: { num: Number(id.slice(1)) },
});

describe("which task each agent pane holds", () => {
    it("maps a claimed or blocked task to its holder's pane, by UID, else by name; the latest when several", () => {
        const board = {
            agents: [
                { name: "A", uid: "uid-a", block_id: "b-a", state: "working" },
                { name: "B", uid: null, block_id: "b-b", state: "idle" },
            ],
            tasks: [
                task("w1", "claimed", "A", "uid-a", 10),
                task("w2", "blocked", "A", "uid-a", 20),
                task("w3", "claimed", "B", "", 5),
                task("w4", "review", "B", "", 50),
                task("w5", "open", "", "", 60),
                task("w6", "claimed", "Gone", "uid-gone", 70),
            ],
            stamp: 0,
            now: 0,
        } as unknown as KanbanBoard;
        const held = heldByBlock(board);
        expect(held.get("b-a")).toEqual({ id: "w2", num: 2, title: "task w2", state: "blocked" });
        expect(held.get("b-b")?.id).toBe("w3");
        expect(held.size).toBe(2);
    });
});
