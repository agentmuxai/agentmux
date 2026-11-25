// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const kanban = vi.hoisted(() => ({ visible: [] as boolean[], tasks: [] as unknown[] }));
vi.mock("../kanban/kanban-model", () => ({
    KanbanModel: class {
        board = () => ({ tasks: kanban.tasks });
        setVisible(on: boolean) {
            kanban.visible.push(on);
        }
    },
}));
vi.mock("@/app/store/auth-headers", () => ({ authHeaders: () => ({ "X-AuthKey": "k" }) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv" }));

import { REFRESH_MS, useAtlasTasks, useAtlasWork } from "./atlas-work-store";

function serve(agents: unknown[]) {
    const fetched = vi.fn(async () => ({ ok: true, json: async () => ({ channel: "stable", agents }) }));
    vi.stubGlobal("fetch", fetched);
    return fetched;
}

/** The hook under a root, with the open card's pane as a signal. */
function mount(initial: string | null) {
    const [block, setBlock] = createSignal<string | null>(initial);
    let work!: ReturnType<typeof useAtlasWork>;
    let tasks!: ReturnType<typeof useAtlasTasks>;
    const dispose = createRoot((d) => {
        tasks = useAtlasTasks();
        work = useAtlasWork(() => (block() ? { blockId: block(), name: "Ann" } : null), tasks);
        return d;
    });
    return { work, tasks, setBlock, dispose };
}

const settle = async () => {
    await vi.advanceTimersByTimeAsync(0);
};

describe("useAtlasWork", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        kanban.visible = [];
        kanban.tasks = [];
    });
    afterEach(() => {
        vi.useRealTimers();
        vi.unstubAllGlobals();
    });

    it("asks srv for nothing with no card open, but follows the Kanban for the nodes' tasks", async () => {
        const fetched = serve([]);
        const { work, dispose } = mount(null);
        await settle();
        expect(fetched).not.toHaveBeenCalled();
        expect(kanban.visible).toEqual([true]);
        expect(work()).toBeNull();
        dispose();
        expect(kanban.visible).toEqual([true, false]);
    });

    it("hands the board's tasks to the nodes", async () => {
        serve([]);
        kanban.tasks = [{ title: "T" }];
        const { tasks, dispose } = mount(null);
        expect(tasks()).toEqual([{ title: "T" }]);
        dispose();
    });

    it("reads the work facts while a card is open, with the tasks it holds", async () => {
        const fetched = serve([{ block_id: "b1", repo: "o/r", branch: "ann/fix" }]);
        kanban.tasks = [{ title: "T", state: "claimed", updated_at: 1, card: { num: 5, repo: "", branch: "", pr_url: "" }, holder: { block_id: "b1" } }];
        const { work, dispose } = mount("b1");
        await settle();
        expect(fetched).toHaveBeenCalledWith("http://srv/api/v1/work-facts", { headers: { "X-AuthKey": "k" } });
        expect(work()).toEqual({
            repo: "o/r",
            branch: "ann/fix",
            tasks: [{ num: 5, title: "T", pr: null, column: "progress", held: true }],
            more: 0,
        });
        dispose();
    });

    it("asks again while the card stays open, and stops when it closes", async () => {
        const fetched = serve([]);
        const { work, setBlock, dispose } = mount("b1");
        await settle();
        await vi.advanceTimersByTimeAsync(REFRESH_MS);
        expect(fetched).toHaveBeenCalledTimes(2);
        setBlock(null);
        await settle();
        expect(work()).toBeNull();
        await vi.advanceTimersByTimeAsync(REFRESH_MS * 3);
        expect(fetched).toHaveBeenCalledTimes(2);
        dispose();
    });

    it("keeps one clock when the card moves to another agent, and stops with its owner", async () => {
        const fetched = serve([]);
        const { setBlock, dispose } = mount("b1");
        await settle();
        setBlock("b2");
        await settle();
        await vi.advanceTimersByTimeAsync(REFRESH_MS);
        expect(fetched).toHaveBeenCalledTimes(2);
        dispose();
        await vi.advanceTimersByTimeAsync(REFRESH_MS * 3);
        expect(fetched).toHaveBeenCalledTimes(2);
        expect(kanban.visible.at(-1)).toBe(false);
    });

    it("shows nothing, and does not fail, on an srv without the route", async () => {
        vi.stubGlobal("fetch", vi.fn(async () => ({ ok: false, json: async () => ({}) })));
        const { work, dispose } = mount("b1");
        await settle();
        expect(work()).toBeNull();
        dispose();
    });
});
