// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Kanban column collapsed to a strip: its name and stats, one button to open it, and the header's
// control to collapse it (operator, 2026-10-28).

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./kanban-card", () => ({
    AgentChip: () => null,
    KanbanCard: () => null,
    UntrackedCard: () => null,
    agentStateWords: () => "",
}));

import { KanbanBoard } from "./kanban-board";
import type { ColumnId } from "./kanban-columns";
import type { KanbanModel } from "./kanban-model";
import type { KanbanTask } from "./kanban-types";

function task(num: number, state: KanbanTask["state"], updated_at: number): KanbanTask {
    return {
        id: `w${num}`,
        title: `task ${num}`,
        payload: "",
        kind: "",
        target_agent: "",
        target_agent_uid: "",
        priority: 0,
        state,
        claimed_by: "",
        claimed_by_uid: "",
        attempts: 0,
        max_attempts: 3,
        created_by: "operator",
        created_at: updated_at,
        updated_at,
        result: "",
        holder: null,
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
            requested_by_kind: "",
            request_text: "",
            filed_by: "",
            updated_at,
        },
    } as KanbanTask;
}

const NOW = 10_000_000;
// Two tasks in Review (the older 2h, the newer 1m) and one on hold in the Backlog (2m).
const tasks = [task(1, "review", NOW - 7_200_000), task(2, "review", NOW - 60_000), task(3, "held", NOW - 120_000)];

function board(over: { collapsed?: ColumnId[]; onToggle?: (id: ColumnId) => void; narrow?: boolean } = {}) {
    return render(() => (
        <KanbanBoard
            tasks={tasks}
            untracked={[]}
            agents={[]}
            model={{ act: vi.fn() } as unknown as KanbanModel}
            now={NOW}
            holding={() => undefined}
            onOpen={() => {}}
            onOpenAgent={() => {}}
            reviewLimit={0}
            reviewTotal={2}
            narrow={over.narrow}
            collapsed={over.collapsed}
            onToggleCollapsed={over.onToggle}
        />
    ));
}

afterEach(cleanup);

describe("a collapsed Kanban column", () => {
    it("is a strip, one button that says the column, its count and what is notable, and opens it", () => {
        const onToggle = vi.fn();
        const { container } = board({ collapsed: ["review"], onToggle });
        const strip = screen.getByRole("button", { name: "Review: 2 tasks, oldest 2h. Expand" });
        expect(strip.getAttribute("aria-expanded")).toBe("false");
        expect(strip.textContent).toContain("Review");
        expect(strip.textContent).toContain("oldest 2h");
        expect(container.querySelector(".kanban-column--review")?.classList.contains("kanban-column--collapsed")).toBe(true);
        // The column's own header is gone while it is a strip, and the others keep theirs.
        expect(screen.queryByRole("button", { name: "Collapse Review" })).toBeNull();
        expect(screen.getByRole("button", { name: "Collapse Backlog" })).toBeTruthy();
        fireEvent.click(strip);
        expect(onToggle).toHaveBeenCalledWith("review");
    });

    it("gives the strip a thin track and the others their usual share", () => {
        const { container } = board({ collapsed: ["review", "merged"], onToggle: vi.fn() });
        const style = (container.querySelector(".kanban-board") as HTMLElement).getAttribute("style") ?? "";
        const tracks = style.replace(/^grid-template-columns:\s*/, "").replace(/;$/, "").split(/ (?![^(]*\))/);
        expect(tracks).toHaveLength(8);
        expect(tracks.filter((t) => t.includes("--kanban-strip-width"))).toHaveLength(2);
        expect(tracks.filter((t) => t.includes("minmax(160px"))).toHaveLength(6);
    });

    it("counts held tasks in the strip's words", () => {
        board({ collapsed: ["backlog"], onToggle: vi.fn() });
        expect(screen.getByRole("button", { name: /Backlog: 1 task, 1 on hold, oldest 2m\. Expand/ })).toBeTruthy();
    });

    it("is collapsed from its header's control", () => {
        const onToggle = vi.fn();
        board({ onToggle });
        fireEvent.click(screen.getByRole("button", { name: "Collapse Review" }));
        expect(onToggle).toHaveBeenCalledWith("review");
    });

    it("has no control, and no strip, where the owner gave none or the list is narrow", () => {
        board({ collapsed: ["review"] });
        expect(screen.queryByRole("button", { name: /Collapse|Expand/ })).toBeNull();
        cleanup();
        board({ collapsed: ["review"], onToggle: vi.fn(), narrow: true });
        expect(screen.queryByRole("button", { name: /Review: .* Expand/ })).toBeNull();
        expect(screen.queryByRole("button", { name: /Collapse Review/ })).toBeNull();
    });
});
