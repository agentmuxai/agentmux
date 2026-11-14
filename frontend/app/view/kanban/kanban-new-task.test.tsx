// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { KanbanNewTask } from "./kanban-new-task";
import type { KanbanModel } from "./kanban-model";

afterEach(cleanup);

describe("the new-task form", () => {
    const fill = async (board: string | undefined) => {
        const act = vi.fn(async () => ({ ok: true }));
        const done = vi.fn();
        render(() => <KanbanNewTask model={{ act } as unknown as KanbanModel} agents={[]} board={board} onDone={done} />);
        fireEvent.input(screen.getByLabelText("Task title"), { target: { value: "Plan it" } });
        fireEvent.click(screen.getByRole("button", { name: "Add task" }));
        await waitFor(() => expect(act).toHaveBeenCalled());
        return act;
    };

    it("files the task on the board the pane shows", async () => {
        const act = await fill("work");
        expect(act).toHaveBeenCalledWith(expect.objectContaining({ action: "create", title: "Plan it", board: "work" }));
    });

    it("names no board when the pane has none, so srv files it on the default", async () => {
        const act = await fill(undefined);
        const req = (act.mock.calls[0] as unknown[])[0] as Record<string, unknown>;
        expect(req.board).toBeUndefined();
    });
});
