// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Taking a task back from an agent that can't work (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md, R5): Release
 * and Reassign in the drawer, with the warning srv gives when the task has a branch or PR.
 */
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListAgentDefinitionsCommand: () => Promise.resolve([]) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/store/misc-utils", () => ({ openLink: vi.fn() }));

import { KanbanDrawer } from "./kanban-drawer";
import type { KanbanModel } from "./kanban-model";
import type { KanbanActRequest, KanbanActResult, KanbanAgent, KanbanTask } from "./kanban-types";

const naki: KanbanAgent = { name: "Naki-2", uid: "uid-n", block_id: "b-n", state: "limited" };
const loap: KanbanAgent = { name: "Loap-3", uid: "uid-l", block_id: "b-l", state: "idle" };

function claimed(over: Partial<KanbanTask> = {}, card: Partial<KanbanTask["card"]> = {}): KanbanTask {
    return {
        id: "w1",
        title: "Fix the thing",
        payload: "do it",
        kind: "",
        target_agent: "",
        target_agent_uid: "",
        priority: 0,
        state: "claimed",
        claimed_by: "Naki-2",
        claimed_by_uid: "uid-n",
        attempts: 1,
        max_attempts: 3,
        created_by: "operator",
        created_at: 1,
        updated_at: 1,
        result: "",
        holder: naki,
        holder_unavailable: { state: "limited", title: "Usage limit reached", resets_at_ms: null, since_ms: 1 },
        card: {
            num: 12,
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
            ...card,
        },
        ...over,
    };
}

function mount(task: KanbanTask, answers: (KanbanActResult | null)[]) {
    const act = vi.fn<(req: KanbanActRequest) => Promise<KanbanActResult | null>>();
    for (const a of answers) act.mockResolvedValueOnce(a);
    const model = { act, detail: () => Promise.resolve({ task, events: [] }), board: () => null } as unknown as KanbanModel;
    render(() => (
        <KanbanDrawer
            model={model}
            task={task}
            agents={[naki, loap]}
            now={1000}
            onClose={() => {}}
            onOpenAgent={() => {}}
            onOpenTask={() => {}}
        />
    ));
    return act;
}

describe("KanbanDrawer: taking a task back", () => {
    beforeEach(() => vi.clearAllMocks());
    afterEach(cleanup);

    it("says the holder can't work and takes the task back in one click", async () => {
        const act = mount(claimed(), [{ ok: true }]);
        expect(screen.getByText(/Naki-2 can't work right now/)).toBeTruthy();
        fireEvent.click(screen.getByText("Take it back"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "release" });
    });

    it("asks before taking back a task with a PR, and repeats the move with the yes", async () => {
        const words = "It has a PR (https://github.com/agentmuxai/agentmux/pull/7). Take it back anyway?";
        const act = mount(claimed({}, { pr_url: "https://github.com/agentmuxai/agentmux/pull/7" }), [{ ok: false, confirm: words }, { ok: true }]);
        fireEvent.click(screen.getByText("Take it back"));
        await screen.findByText(words);
        expect(act).toHaveBeenCalledTimes(1);
        fireEvent.click(screen.getByText("Take it back anyway"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(2));
        expect(act.mock.calls[1][0]).toEqual({ id: "w1", action: "release", confirm: true });
        await waitFor(() => expect(screen.queryByText(words)).toBeNull());
    });

    it("leaves it when the operator says so", async () => {
        const words = "It has a branch (x). Take it back anyway?";
        const act = mount(claimed({}, { branch: "x" }), [{ ok: false, confirm: words }]);
        fireEvent.click(screen.getByText("Take it back"));
        await screen.findByText(words);
        fireEvent.click(screen.getByText("Leave it"));
        expect(screen.queryByText(words)).toBeNull();
        expect(act).toHaveBeenCalledTimes(1);
    });

    it("gives it to another agent, or to anyone, and never offers the holder", async () => {
        const act = mount(claimed(), [{ ok: true }, { ok: true }]);
        const select = screen.getByLabelText("Agent to give it to") as HTMLSelectElement;
        const labels = [...select.options].map((o) => o.textContent);
        expect(labels).toContain("Anyone");
        expect(labels.some((l) => l?.startsWith("Loap-3"))).toBe(true);
        expect(labels.some((l) => l?.startsWith("Naki-2"))).toBe(false);
        fireEvent.change(select, { target: { value: "Loap-3" } });
        fireEvent.click(screen.getByText("Give"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "reassign", agent: "Loap-3" });
        fireEvent.change(select, { target: { value: "__anyone" } });
        fireEvent.click(screen.getByText("Give"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(2));
        expect(act.mock.calls[1][0]).toEqual({ id: "w1", action: "reassign", agent: "" });
    });

    it("offers nothing to take back on a task nobody holds", () => {
        mount(claimed({ state: "open", claimed_by: "", claimed_by_uid: "", holder: null, holder_unavailable: null }), []);
        expect(screen.queryByText("Take it back")).toBeNull();
    });
});

describe("KanbanDrawer: a leader's acceptance", () => {
    beforeEach(() => vi.clearAllMocks());
    afterEach(cleanup);

    const accepted = (over: Partial<KanbanTask> = {}) =>
        claimed({ state: "done", holder: null, holder_unavailable: null, accepted_by: { name: "AgentY", said: "accept them", at: 5 }, ...over });

    it("lets the operator reopen what a leader accepted on their word", async () => {
        const act = mount(accepted(), [{ ok: true }]);
        fireEvent.click(screen.getByText("Reopen"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "reopen", text: "" });
    });

    it("offers no Reopen for a task the operator accepted themselves", () => {
        mount(accepted({ accepted_by: null }), []);
        expect(screen.queryByText("Reopen")).toBeNull();
    });
});
