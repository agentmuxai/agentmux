// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { agentChoices, agentEntries } from "./kanban-boards-panel";
import { moveEntries } from "./kanban-board-picker";
import {
    boardSlugOf,
    defaultSlugOf,
    needsByBoard,
    refOf,
    resolveBoard,
    secondsLeft,
    tasksOnLiveBoards,
    tierOf,
    viewOfBoard,
} from "./kanban-boards";
import type { KanbanBoard, KanbanBoardInfo } from "./kanban-types";

const info = (slug: string, more: Partial<KanbanBoardInfo> = {}): KanbanBoardInfo => ({
    id: `id-${slug}`,
    slug,
    name: slug.toUpperCase(),
    tier: "local",
    readable: false,
    agents: [],
    default: false,
    archived: false,
    open: 0,
    total: 0,
    ...more,
});

const task = (id: string, board: string | undefined, state = "open", num = 1) => ({
    id,
    state,
    title: id,
    kind: "",
    card: { num, board, task_ref: board && board !== "main" ? `${board}#${num}` : `#${num}` },
});

const raw = (extra: Partial<KanbanBoard> = {}): KanbanBoard =>
    ({
        tasks: [task("a", "main", "blocked"), task("b", "work", "blocked", 2), task("c", "work", "open", 3), task("d", "old", "failed", 4)],
        agents: [],
        boards: [info("main", { default: true }), info("work"), info("old", { archived: true })],
        board: "all",
        default_board: "main",
        stamp: 1,
        now: 1,
        ...extra,
    }) as unknown as KanbanBoard;

describe("which board the pane shows", () => {
    it("shows the selected board's tasks and keeps every board's info", () => {
        const view = viewOfBoard(raw(), "work")!;
        expect(view.tasks.map((t) => t.id)).toEqual(["b", "c"]);
        expect(view.board).toBe("work");
        expect(view.boards).toHaveLength(3);
    });

    it("shows the default board when none is selected, or the selected one is gone", () => {
        expect(viewOfBoard(raw(), null)!.tasks.map((t) => t.id)).toEqual(["a"]);
        expect(viewOfBoard(raw(), "deleted")!.board).toBe("main");
        expect(resolveBoard(raw().boards!, "work", "main")).toBe("work");
    });

    it("takes the default from srv, and puts a task with no board on it", () => {
        expect(defaultSlugOf(raw({ default_board: "work" }))).toBe("work");
        expect(defaultSlugOf(raw({ default_board: undefined }))).toBe("main");
        expect(defaultSlugOf({ tasks: [], agents: [] } as unknown as KanbanBoard)).toBe("main");
        const old = viewOfBoard({ tasks: [task("x", undefined)], agents: [], stamp: 1, now: 1 } as unknown as KanbanBoard, null)!;
        expect(old.tasks).toHaveLength(1);
        expect(boardSlugOf(task("x", undefined) as never, "main")).toBe("main");
    });

    it("shows nothing before the first answer", () => {
        expect(viewOfBoard(null, "work")).toBeNull();
    });
});

describe("what waits for the operator", () => {
    it("counts each board's own, and the title counts the boards that aren't archived", () => {
        expect(needsByBoard(raw())).toEqual({ main: 1, work: 1, old: 1 });
        expect(tasksOnLiveBoards(raw()).map((t) => t.id)).toEqual(["a", "b", "c"]);
    });
});

describe("how a task is named", () => {
    it("is #n on the default board and slug#n elsewhere", () => {
        expect(refOf(task("a", "main") as never)).toBe("#1");
        expect(refOf(task("b", "work", "open", 2) as never)).toBe("work#2");
        expect(refOf({ card: { num: 7 } } as never)).toBe("#7");
    });
});

describe("the picker and the panel", () => {
    it("move to board lists the live boards the task isn't on", () => {
        const picked: string[] = [];
        const entries = moveEntries(raw().boards!, "main", (b) => picked.push(b.slug));
        expect(entries.map((e) => e.label)).toEqual(["WORK"]);
        entries[0].onSelect?.();
        expect(picked).toEqual(["work"]);
    });

    it("limits a board to the agents ticked, and All agents clears the list", () => {
        const seen: string[][] = [];
        const entries = agentEntries(agentChoices(["Zed", "Amy"], ["Amy", "Gone"]), ["Amy"], (a) => seen.push(a));
        expect(entries.filter((e) => !e.divider).map((e) => [e.label, e.checked])).toEqual([
            ["All agents", false],
            ["Amy", true],
            ["Gone", false],
            ["Zed", false],
        ]);
        entries[0].onSelect?.();
        entries.find((e) => e.label === "Zed")!.onSelect?.();
        entries.find((e) => e.label === "Amy")!.onSelect?.();
        expect(seen).toEqual([[], ["Amy", "Zed"], []]);
    });

    it("counts a delete down to zero and names the tier", () => {
        expect(secondsLeft(10_000, 4_500)).toBe(6);
        expect(secondsLeft(10_000, 20_000)).toBe(0);
        expect(secondsLeft(null, 1)).toBe(0);
        expect(tierOf("local").label).toBe("This machine only");
        expect(tierOf("lan").icon).toBe("network-wired");
    });
});
