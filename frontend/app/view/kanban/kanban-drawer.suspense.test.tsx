// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Clicking through tasks must not repaint the whole pane. The pane sits inside the block's `<Suspense>`,
 * and a pending `createResource` read under it swaps the pane for the block's spinner: the board is
 * detached and comes back scrolled to the top. The drawer's history waits for its own fetch, so the
 * fetch must never reach that boundary: the board element stays the same node, its scroll stays, and the
 * pane-level fallback is never shown, while a fetch is pending for a first, a second and a third task.
 */
import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal, Suspense } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListAgentDefinitionsCommand: () => Promise.resolve([]) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/store/misc-utils", () => ({ openLink: vi.fn() }));

import { KanbanDrawer } from "./kanban-drawer";
import type { KanbanModel } from "./kanban-model";
import type { KanbanEvent, KanbanTask, KanbanTaskDetail } from "./kanban-types";

function waiting(num: number, over: Partial<KanbanTask> = {}): KanbanTask {
    return {
        id: `w${num}`,
        title: `Task ${num}`,
        payload: "do it",
        kind: "",
        target_agent: "",
        target_agent_uid: "",
        priority: 0,
        state: "open",
        claimed_by: "",
        claimed_by_uid: "",
        attempts: 0,
        max_attempts: 3,
        created_by: "operator",
        created_at: 1,
        updated_at: 1,
        result: "",
        holder: undefined,
        card: {
            num,
            repo: "",
            branch: "",
            pr_url: "",
            review_required: false,
            checklist: [],
            question: "",
            note: "",
            requested_by: "",
            requested_by_kind: "operator",
            request_text: "",
            filed_by: "",
            updated_at: 1,
        },
        ...over,
    };
}

function event(id: number, body: string): KanbanEvent {
    return { id, work_id: "", kind: "filed", actor: "operator", actor_kind: "operator", from_state: "", to_state: "open", body, at: 1 };
}

/** A model whose `detail` answers only when the test says so. */
function slowModel() {
    const waits = new Map<string, (d: KanbanTaskDetail | null) => void>();
    const asked: string[] = [];
    const model = {
        detail: (id: string) => {
            asked.push(id);
            return new Promise<KanbanTaskDetail | null>((res) => waits.set(id, res));
        },
        board: () => null,
    } as unknown as KanbanModel;
    const answer = (task: KanbanTask, history: string) => waits.get(task.id)?.({ task, events: [event(Number(task.card.num), history)] });
    return { model, answer, asked };
}

function mount(first: KanbanTask) {
    const { model, answer, asked } = slowModel();
    const [task, setTask] = createSignal(first);
    render(() => (
        <Suspense fallback={<div data-testid="pane-fallback" />}>
            <div data-testid="board" />
            <KanbanDrawer
                model={model}
                task={task()}
                agents={[]}
                now={1000}
                onClose={() => {}}
                onOpenAgent={() => {}}
                onOpenTask={() => {}}
            />
        </Suspense>
    ));
    return { setTask, answer, asked };
}

afterEach(cleanup);

describe("the drawer's fetch under the pane's Suspense", () => {
    it("never swaps the pane for its fallback, opening a task or switching between three", async () => {
        const t1 = waiting(1);
        const t2 = waiting(2);
        const t3 = waiting(3);
        const { setTask, answer } = mount(t1);

        // The first task's history is still on its way: the board is still there, the fallback is not.
        const board = screen.getByTestId("board");
        expect(screen.queryByTestId("pane-fallback")).toBeNull();
        expect(screen.getByText("Loading…")).toBeTruthy();

        answer(t1, "hist-one");
        await screen.findByText(/hist-one$/);
        expect(screen.getByTestId("board")).toBe(board);

        for (const [next, history] of [[t2, "hist-two"], [t3, "hist-three"]] as const) {
            setTask(next);
            // While the next task's history is pending.
            expect(screen.queryByTestId("pane-fallback")).toBeNull();
            expect(screen.getByTestId("board")).toBe(board);
            expect(screen.getByText(`Task ${next.card.num}`)).toBeTruthy();
            // The previous task's history is not shown under the new task.
            expect(screen.queryByText(/history of (one|two)/)).toBeNull();
            answer(next, history);
            await screen.findByText(new RegExp(history));
            expect(screen.getByTestId("board")).toBe(board);
        }
    });

    it("keeps the history on screen while the same task is read again after a board refresh", async () => {
        const t1 = waiting(1);
        const { setTask, answer, asked } = mount(t1);
        answer(t1, "hist-one");
        await screen.findByText(/hist-one$/);

        setTask(waiting(1, { updated_at: 2 }));
        await waitFor(() => expect(asked).toEqual(["w1", "w1"]));
        // The refetch is pending: the old history stays and the pane is not swapped.
        expect(screen.getByText(/hist-one$/)).toBeTruthy();
        expect(screen.queryByTestId("pane-fallback")).toBeNull();
        answer(waiting(1, { updated_at: 2 }), "hist-one-newer");
        await screen.findByText(/hist-one-newer/);
    });
});
