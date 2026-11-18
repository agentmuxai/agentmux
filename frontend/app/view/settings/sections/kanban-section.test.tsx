// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings → Kanban's stage editor over a real `KanbanModel` (srv's answers stubbed): the default eight, a rename
 * that Apply sends with the revision it read, the rules that keep Apply off, removing a stage into another, Add at
 * the limit, and Reset. The editor edits a draft, so nothing is sent before Apply.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { defaultStages, toStageSet } from "@/app/view/kanban/kanban-stages";

const h = vi.hoisted(() => ({ acts: [] as any[], board: null as any, actFails: "", onQueue: null as null | (() => void) }));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        KanbanListCommand: async () => h.board,
        KanbanActCommand: async (_c: unknown, req: unknown) => {
            h.acts.push(req);
            if (h.actFails) throw new Error(h.actFails);
            return { ok: true };
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_e: unknown, f: () => void) => {
        h.onQueue = f;
        return () => {};
    },
}));

import { KanbanSection } from "./kanban-section";

function info(slug: string, over: Record<string, unknown> = {}) {
    return {
        id: slug,
        slug,
        name: slug === "main" ? "Main" : "Work",
        tier: "local",
        readable: true,
        agents: [],
        default: slug === "main",
        archived: false,
        open: 0,
        total: 0,
        stages: toStageSet(defaultStages()),
        stages_rev: 0,
        stages_default: true,
        stages_error: null,
        ...over,
    };
}

function boardOf(boards: unknown[]) {
    return { tasks: [], agents: [], boards, board: "all", default_board: "main", stamp: 1, now: 1 };
}

const names = () => screen.getAllByLabelText("Stage name").map((el) => (el as HTMLInputElement).value);
const apply = () => screen.getByRole("button", { name: "Apply" }) as HTMLButtonElement;

async function shown() {
    render(() => <KanbanSection />);
    await screen.findByDisplayValue("Backlog");
}

beforeEach(() => {
    h.acts = [];
    h.actFails = "";
    h.onQueue = null;
    h.board = boardOf([info("main"), info("work")]);
});
afterEach(cleanup);

describe("Settings → Kanban", () => {
    it("shows the default eight stages with nothing to apply", async () => {
        await shown();
        expect(names()).toEqual(["Backlog", "In Research", "Spec'd", "In progress", "Needs you", "Review", "Merged", "Released"]);
        expect(apply().disabled).toBe(true);
        expect(h.acts).toEqual([]);
    });

    it("sends a rename with the revision it read, only when applied", async () => {
        await shown();
        fireEvent.input(screen.getAllByLabelText("Stage name")[0], { target: { value: "Waiting" } });
        expect(apply().disabled).toBe(false);
        expect(h.acts).toEqual([]);
        fireEvent.click(apply());
        await waitFor(() => expect(h.acts).toHaveLength(1));
        expect(h.acts[0]).toMatchObject({ action: "board_edit", board: "main", stages_rev: 0 });
        expect(h.acts[0].stages.stages[0]).toMatchObject({ id: "backlog", name: "Waiting", colour: "grey", takes: ["backlog"] });
    });

    it("keeps Apply off, and says why, while a name is empty or taken", async () => {
        await shown();
        const inputs = screen.getAllByLabelText("Stage name");
        fireEvent.input(inputs[0], { target: { value: "  " } });
        expect(apply().disabled).toBe(true);
        expect(screen.getAllByText("Give it a name").length).toBeGreaterThan(0);
        fireEvent.input(inputs[0], { target: { value: "merged" } });
        expect(apply().disabled).toBe(true);
        expect(screen.getAllByText("Another stage has this name").length).toBeGreaterThan(0);
    });

    it("never offers to remove the stage that shows what needs you, and says why", async () => {
        await shown();
        expect((screen.getByRole("button", { name: "Can't be removed" }) as HTMLButtonElement).disabled).toBe(true);
        expect(screen.getByText("Shows what needs you, so it always stays a stage of its own.")).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Remove Needs you" })).toBeNull();
        expect(screen.getByRole("button", { name: "Remove Backlog" })).toBeTruthy();
    });

    it("removes a stage into the one before it, and the stages it sends show its sources there", async () => {
        await shown();
        fireEvent.click(screen.getByRole("button", { name: "Remove Released" }));
        fireEvent.click(screen.getByRole("button", { name: "Remove stage" }));
        expect(names()).toHaveLength(7);
        fireEvent.click(apply());
        await waitFor(() => expect(h.acts).toHaveLength(1));
        const merged = h.acts[0].stages.stages.find((s: any) => s.id === "merged");
        expect(merged.takes).toEqual(["merged", "released"]);
        expect(h.acts[0].stages.stages).toHaveLength(7);
    });

    it("lets a stage be added until the board has eight", async () => {
        await shown();
        const add = screen.getByRole("button", { name: "Add stage" }) as HTMLButtonElement;
        expect(add.disabled).toBe(true);
        fireEvent.click(screen.getByRole("button", { name: "Remove Released" }));
        fireEvent.click(screen.getByRole("button", { name: "Remove stage" }));
        expect((screen.getByRole("button", { name: "Add stage" }) as HTMLButtonElement).disabled).toBe(false);
        fireEvent.click(screen.getByRole("button", { name: "Add stage" }));
        expect(names()).toHaveLength(8);
        expect(apply().disabled).toBe(true);
    });

    it("resets a custom set to the default with stages_reset", async () => {
        const custom = defaultStages().filter((s) => s.id !== "released");
        custom[custom.length - 1] = { ...custom[custom.length - 1], takes: ["merged", "released"], name: "Done" };
        h.board = boardOf([info("main", { stages: toStageSet(custom), stages_rev: 3, stages_default: false })]);
        render(() => <KanbanSection />);
        await screen.findByDisplayValue("Done");
        fireEvent.click(screen.getAllByRole("button", { name: "Reset to default" })[0]);
        fireEvent.click(apply());
        await waitFor(() => expect(h.acts).toHaveLength(1));
        expect(h.acts[0]).toMatchObject({ action: "board_edit", board: "main", stages_reset: true, stages_rev: 3 });
        expect(h.acts[0].stages).toBeUndefined();
    });

    it("shows why Apply was refused and keeps the draft", async () => {
        await shown();
        h.actFails = "the stages changed since you read them (now revision 2)";
        fireEvent.input(screen.getAllByLabelText("Stage name")[0], { target: { value: "Waiting" } });
        fireEvent.click(apply());
        await screen.findByText("the stages changed since you read them (now revision 2)");
        // The board is read again after a refusal; the reason stays on screen when that answer arrives.
        await new Promise((r) => setTimeout(r, 100));
        expect(screen.getByText("the stages changed since you read them (now revision 2)")).toBeTruthy();
        expect(names()[0]).toBe("Waiting");
        await waitFor(() => expect(apply().disabled).toBe(false));
    });

    it("keeps the draft when the board is read again with nothing changed", async () => {
        await shown();
        fireEvent.input(screen.getAllByLabelText("Stage name")[0], { target: { value: "Waiting" } });
        // The queue changed (or the poll ran): srv answers with a fresh object for the same boards.
        h.board = { ...boardOf([info("main"), info("work")]), stamp: 2, now: 2 };
        h.onQueue?.();
        await new Promise((r) => setTimeout(r, 400));
        expect(names()[0]).toBe("Waiting");
        expect(apply().disabled).toBe(false);
    });

    it("lets a new stage take a source by its chip, and sends it", async () => {
        await shown();
        fireEvent.click(screen.getByRole("button", { name: "Remove Released" }));
        fireEvent.click(screen.getByRole("button", { name: "Remove stage" }));
        fireEvent.click(screen.getByRole("button", { name: "Add stage" }));
        expect(apply().disabled).toBe(true);
        const rows = screen.getAllByRole("listitem");
        const chip = Array.from(rows[rows.length - 1].querySelectorAll("button")).find((b) => b.textContent === "Merged") as HTMLElement;
        fireEvent.click(chip);
        expect(apply().disabled).toBe(false);
        fireEvent.click(apply());
        await waitFor(() => expect(h.acts).toHaveLength(1));
        expect(h.acts[0].stages.stages.map((s: any) => s.takes)).toContainEqual(["merged"]);
    });

    it("says an archived board's stages can't be changed", async () => {
        h.board = boardOf([info("main"), info("old", { archived: true })]);
        render(() => <KanbanSection model={{ kanbanBoard: () => "old" } as never} />);
        await screen.findByText("This board is archived, so its stages can't be changed.");
        expect((screen.getAllByLabelText("Stage name")[0] as HTMLInputElement).disabled).toBe(true);
    });
});
