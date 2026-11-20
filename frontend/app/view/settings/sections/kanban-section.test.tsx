// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings → Kanban's stage editor over a real `KanbanModel` (srv's answers stubbed): the default eight, a rename
 * that Apply sends with the revision it read, the rules that keep Apply off, removing a stage into another, Add at
 * the limit, and Reset. The editor edits a draft, so nothing is sent before Apply.
 */

import { cleanup, createEvent, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
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
afterEach(() => {
    cleanup();
    document.querySelectorAll(".kanban-stages__ghost").forEach((g) => g.remove());
});

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
        expect(h.acts[0].stages.stages[0]).toMatchObject({ id: "backlog", name: "Waiting", colour: "muted", takes: ["backlog"] });
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

    it("lists who changed the stages and when, newest first, and names the operator as You", async () => {
        const lines = [
            { at: 2000, actor: "", actor_kind: "operator", kind: "stages", body: "reset the stages to the default: 5 → 8" },
            { at: 1000, actor: "agentx", actor_kind: "agent", kind: "stages", body: "changed the stages: 8 → 5" },
        ];
        h.board = boardOf([info("main", { history: lines })]);
        await shown();
        const items = screen.getAllByRole("listitem").filter((li) => li.textContent?.includes("the stages"));
        expect(items).toHaveLength(2);
        expect(items[0].textContent).toContain("You reset the stages to the default: 5 → 8");
        expect(items[1].textContent).toContain("agentx changed the stages: 8 → 5");
    });

    it("shows no history list for a board nobody has changed", async () => {
        await shown();
        expect(screen.queryByText("Recent changes")).toBeNull();
    });

    it("reorders a stage by dragging its grip onto another row, before or after by the pointer", async () => {
        await shown();
        const rows = () => Array.from(document.querySelectorAll<HTMLElement>(".kanban-stages__row"));
        const grip = (i: number) => rows()[i].querySelector<HTMLElement>(".kanban-stages__grip") as HTMLElement;
        const data: Record<string, string> = {};
        const dataTransfer = { setData: (k: string, v: string) => (data[k] = v), setDragImage: () => {}, effectAllowed: "", dropEffect: "" };
        // jsdom has no DragEvent, so the pointer's height is put on the event by hand (the rows have no size here).
        const over = (el: HTMLElement, clientY: number) => {
            const ev = createEvent.dragOver(el, { dataTransfer });
            Object.defineProperty(ev, "clientY", { value: clientY });
            fireEvent(el, ev);
        };
        // Review (index 5) onto Backlog (0), upper half: it lands first.
        fireEvent.dragStart(grip(5), { dataTransfer });
        expect(rows()[5].hasAttribute("data-dragging")).toBe(true);
        over(rows()[0], -1);
        expect(rows()[0].getAttribute("data-drop")).toBe("before");
        fireEvent.drop(rows()[0], { dataTransfer });
        fireEvent.dragEnd(grip(5), { dataTransfer });
        expect(names().slice(0, 3)).toEqual(["Review", "Backlog", "In Research"]);
        expect(document.querySelector("[data-dragging]")).toBeNull();
        expect(document.querySelector("[data-drop]")).toBeNull();
        // Review (now 0) onto Released (7), lower half: it lands last.
        fireEvent.dragStart(grip(0), { dataTransfer });
        over(rows()[7], 10);
        expect(rows()[7].getAttribute("data-drop")).toBe("after");
        fireEvent.drop(rows()[7], { dataTransfer });
        expect(names()[7]).toBe("Review");
        expect(apply().disabled).toBe(false);
        fireEvent.click(apply());
        await waitFor(() => expect(h.acts).toHaveLength(1));
        expect(h.acts[0].stages.stages.map((s: any) => s.id)).toEqual([
            "backlog", "research", "specd", "progress", "needs", "merged", "released", "review",
        ]);
    });

    it("drags a copy of the row on a solid surface, with what was typed, and takes it away at once", async () => {
        await shown();
        const rows = () => Array.from(document.querySelectorAll<HTMLElement>("li.kanban-stages__row:not(.kanban-stages__ghost)"));
        fireEvent.input(screen.getAllByLabelText("Stage name")[1], { target: { value: "Looking" } });
        let image: HTMLElement | undefined;
        const grip = rows()[1].querySelector<HTMLElement>(".kanban-stages__grip") as HTMLElement;
        fireEvent.dragStart(grip, { dataTransfer: { setData: () => {}, setDragImage: (el: HTMLElement) => { image = el; } } });
        expect(image).toBeDefined();
        expect(image).not.toBe(rows()[1]);
        expect(image!.classList.contains("kanban-stages__ghost")).toBe(true);
        expect(image!.isConnected).toBe(true);
        expect((image!.querySelector("input") as HTMLInputElement).value).toBe("Looking");
        fireEvent.dragEnd(grip);
        expect(image!.isConnected).toBe(false);
    });

    it("ignores a drop that did not start from a grip, and a drop on the held row", async () => {
        await shown();
        const rows = () => Array.from(document.querySelectorAll<HTMLElement>(".kanban-stages__row"));
        fireEvent.dragOver(rows()[2], { clientY: 10 });
        fireEvent.drop(rows()[2]);
        const grip = rows()[1].querySelector<HTMLElement>(".kanban-stages__grip") as HTMLElement;
        fireEvent.dragStart(grip, { dataTransfer: { setData: () => {}, setDragImage: () => {} } });
        fireEvent.drop(rows()[1]);
        expect(apply().disabled).toBe(true);
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

    it("shows only the sentence of a stale refusal, not the stage set srv adds for an agent's retry", async () => {
        await shown();
        h.actFails =
            'the stages changed since you read them (now revision 2). The stages are now {"version":1,"stages":[]}. Apply your change to that and send it with stages_rev 2.';
        fireEvent.input(screen.getAllByLabelText("Stage name")[0], { target: { value: "Waiting" } });
        fireEvent.click(apply());
        await screen.findByText("The stages changed since you read them (now revision 2).");
        expect(document.body.textContent).not.toContain('"version":1');
    });

    it("shows why Apply was refused and keeps the draft", async () => {
        await shown();
        h.actFails = "the stages changed since you read them (now revision 2)";
        fireEvent.input(screen.getAllByLabelText("Stage name")[0], { target: { value: "Waiting" } });
        fireEvent.click(apply());
        await screen.findByText("The stages changed since you read them (now revision 2)");
        // The board is read again after a refusal; the reason stays on screen when that answer arrives.
        await new Promise((r) => setTimeout(r, 100));
        expect(screen.getByText("The stages changed since you read them (now revision 2)")).toBeTruthy();
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

    it("lets Reset repair a board whose stored stages can't be used, by sending the reset", async () => {
        h.board = boardOf([info("main", { stages_error: "version 9 is not one this AgentMux reads", stages_rev: 4, stages_default: false })]);
        render(() => <KanbanSection />);
        await screen.findByText(/can't be used/);
        expect(apply().disabled).toBe(true);
        fireEvent.click(screen.getAllByRole("button", { name: "Reset to default" })[0]);
        expect(apply().disabled).toBe(false);
        fireEvent.click(apply());
        await waitFor(() => expect(h.acts).toHaveLength(1));
        expect(h.acts[0]).toMatchObject({ action: "board_edit", board: "main", stages_reset: true, stages_rev: 4 });
    });

    it("goes back to the cog's board on a second ask for it, after another board was picked", async () => {
        const [ask, setAsk] = createSignal(1);
        render(() => <KanbanSection model={{ kanbanBoard: () => "main", kanbanAsk: ask } as never} />);
        await screen.findByDisplayValue("Backlog");
        const board = () => screen.getByLabelText("Board") as HTMLSelectElement;
        fireEvent.change(board(), { target: { value: "work" } });
        await waitFor(() => expect(board().value).toBe("work"));
        setAsk(2);
        await waitFor(() => expect(board().value).toBe("main"));
    });

    it("says an archived board's stages can't be changed", async () => {
        h.board = boardOf([info("main"), info("old", { archived: true })]);
        render(() => <KanbanSection model={{ kanbanBoard: () => "old" } as never} />);
        await screen.findByText("This board is archived, so its stages can't be changed.");
        expect((screen.getAllByLabelText("Stage name")[0] as HTMLInputElement).disabled).toBe(true);
    });
});
