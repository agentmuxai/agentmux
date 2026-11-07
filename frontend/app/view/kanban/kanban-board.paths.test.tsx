// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Kanban board (KanbanBoard in kanban-board.tsx), path by path: which tasks sit in which column and
// what each column's header says, the narrow list's folded columns, the collapse strips, the Backlog and
// Spec'd groups and what a dropped card does there, Review's load, the untracked agents in In progress and
// the cancelled tasks under Merged. The cards are stubs, and the drop targets are recorded rather than
// dragged to.

import { cleanup, fireEvent, render, screen, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { targets } = vi.hoisted(() => ({
    targets: [] as {
        element: HTMLElement;
        canDrop: (a: { source: { data: Record<string, unknown> } }) => boolean;
        onDragEnter: () => void;
        onDragLeave: () => void;
        onDrop: (a: { source: { data: Record<string, unknown> } }) => void;
    }[],
}));
vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    dropTargetForElements: (cfg: (typeof targets)[number]) => {
        targets.push(cfg);
        return () => {
            const i = targets.indexOf(cfg);
            if (i >= 0) targets.splice(i, 1);
        };
    },
}));

vi.mock("./kanban-card", () => ({
    AgentChip: (p: {
        name: string;
        uid?: string | null;
        agent?: { name: string };
        colors?: Record<string, string>;
    }) => (
        <span
            data-testid="chip"
            data-name={p.name}
            data-uid={p.uid ?? ""}
            data-agent={p.agent?.name ?? ""}
            data-colors={JSON.stringify(p.colors ?? null)}
        />
    ),
    KanbanCard: (p: {
        task: KanbanTask;
        now: number;
        agents: KanbanAgent[];
        colors?: Record<string, string>;
        onOpen: (t: KanbanTask) => void;
    }) => (
        <div
            data-testid={`card-${p.task.card.num}`}
            data-now={p.now}
            data-agents={p.agents.map((a) => a.name).join(",")}
            data-colors={JSON.stringify(p.colors ?? null)}
            onClick={() => p.onOpen(p.task)}
        />
    ),
    UntrackedCard: (p: {
        agent: KanbanAgent;
        onTrack: (a: KanbanAgent) => void;
        onOpenAgent: (a: KanbanAgent) => void;
    }) => (
        <div data-testid={`untracked-${p.agent.block_id}`}>
            <button data-testid={`track-${p.agent.block_id}`} onClick={() => p.onTrack(p.agent)} />
            <button data-testid={`open-${p.agent.block_id}`} onClick={() => p.onOpenAgent(p.agent)} />
        </div>
    ),
    agentStateWords: (s: string) => `state:${s}`,
}));

import { kanbanCardItemType } from "@/app/drag/drag-types";
import { KanbanBoard } from "./kanban-board";
import type { ColumnId } from "./kanban-columns";
import type { KanbanModel } from "./kanban-model";
import type { KanbanAgent, KanbanCardFields, KanbanTask } from "./kanban-types";

const NOW = 100_000_000;
const MIN = 60_000;

function task(
    num: number,
    state: KanbanTask["state"],
    over: Partial<KanbanTask> = {},
    card: Partial<KanbanCardFields> = {}
): KanbanTask {
    const updated_at = over.updated_at ?? NOW - num * MIN;
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
        holder: null,
        result: "",
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
            ...card,
        },
        ...over,
        updated_at,
    } as KanbanTask;
}

const agent = (name: string, over: Partial<KanbanAgent> = {}): KanbanAgent => ({
    name,
    uid: `uid-${name}`,
    block_id: `block-${name}`,
    state: "idle",
    ...over,
});

const working = agent("Worker", { state: "working" });
const spec = { id: "s1", num: 98, title: "the spec", state: "done" as const, kind: "spec" };

interface Props {
    tasks: KanbanTask[];
    untracked: KanbanAgent[];
    agents: KanbanAgent[];
    holding: (agent: string) => KanbanTask | undefined;
    reviewLimit: number;
    reviewTotal: number;
    colors?: Record<string, string>;
    narrow?: boolean;
    collapsed?: readonly ColumnId[];
    onToggleCollapsed?: (id: ColumnId) => void;
}

function mount(over: Partial<Props> = {}) {
    const act = vi.fn(() => Promise.resolve());
    const onOpen = vi.fn();
    const onOpenAgent = vi.fn();
    const props: Props = {
        tasks: [],
        untracked: [],
        agents: [],
        holding: () => undefined,
        reviewLimit: 0,
        reviewTotal: 0,
        ...over,
    };
    const result = render(() => (
        <KanbanBoard
            {...props}
            model={{ act } as unknown as KanbanModel}
            now={NOW}
            onOpen={onOpen}
            onOpenAgent={onOpenAgent}
        />
    ));
    return { ...result, act, onOpen, onOpenAgent };
}

const column = (label: string) => screen.getByRole("region", { name: label });
const head = (label: string) => column(label).querySelector(".kanban-column__head") as HTMLElement;
const count = (label: string) => column(label).querySelector(".kanban-column__count") as HTMLElement;
const countText = (label: string) => (count(label).textContent ?? "").replace(/\s+/g, " ").trim();
const cards = (label: string) =>
    Array.from(column(label).querySelectorAll("[data-testid^=card-]")).map((c) => c.getAttribute("data-testid"));
const dropAt = (el: Element) => targets.find((t) => t.element === el || el.contains(t.element))!;
const dragged = (id: string) => ({ source: { data: { type: kanbanCardItemType, id } } });

beforeEach(() => {
    targets.length = 0;
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe("KanbanBoard: the columns", () => {
    it("has the eight columns in order, each labelled", () => {
        mount();
        const labels = screen.getAllByRole("region").map((s) => s.getAttribute("aria-label"));
        expect(labels).toEqual([
            "Backlog",
            "In Research",
            "Spec'd",
            "In progress",
            "Needs you",
            "Review",
            "Merged",
            "Released",
        ]);
        const ids = ["backlog", "research", "specd", "progress", "needs", "review", "merged", "released"];
        for (const [i, id] of ids.entries()) {
            expect(screen.getAllByRole("region")[i]).toHaveClass("kanban-column", `kanban-column--${id}`);
        }
    });

    it("says each column's name in its header", () => {
        mount();
        const labels = screen.getAllByRole("region").map((s) => s.querySelector(".kanban-column__label")?.textContent);
        expect(labels).toEqual([
            "Backlog",
            "In Research",
            "Spec'd",
            "In progress",
            "Needs you",
            "Review",
            "Merged",
            "Released",
        ]);
    });

    it("puts each task in the column its state says, newest change first", () => {
        mount({
            tasks: [
                task(1, "open"),
                task(2, "open", {}, { after: [spec] }),
                task(3, "claimed", { kind: "spec", holder: working }),
                task(4, "claimed", { holder: working }),
                task(5, "claimed", { holder: working, updated_at: NOW - 10 * MIN }),
                task(6, "blocked"),
                task(7, "review"),
                task(8, "done"),
                task(9, "done", {}, { pr: { released_in: "v1" } }),
            ],
        });
        expect(cards("Backlog")).toEqual(["card-1"]);
        expect(cards("Spec'd")).toEqual(["card-2"]);
        expect(cards("In Research")).toEqual(["card-3"]);
        // 4 changed 4 minutes ago, 5 ten minutes ago: 4 is first.
        expect(cards("In progress")).toEqual(["card-4", "card-5"]);
        expect(cards("Needs you")).toEqual(["card-6"]);
        expect(cards("Review")).toEqual(["card-7"]);
        expect(cards("Merged")).toEqual(["card-8"]);
        expect(cards("Released")).toEqual(["card-9"]);
    });

    it("counts a column's tasks in its header", () => {
        mount({
            tasks: [
                task(4, "claimed", { holder: working }),
                task(5, "claimed", { holder: working }),
                task(6, "blocked"),
            ],
        });
        expect(countText("In progress")).toBe("2");
        expect(countText("Needs you")).toBe("1");
        expect(countText("Merged")).toBe("0");
    });

    it("counts the agents working with no task in In progress, and shows their cards there", () => {
        const idle = agent("Idle", { block_id: "b-idle" });
        const { act, onOpenAgent } = mount({
            tasks: [task(4, "claimed", { holder: working })],
            untracked: [idle, agent("Other", { block_id: "b-other" })],
        });
        expect(countText("In progress")).toBe("3");
        expect(countText("Needs you")).toBe("0");
        const progress = within(column("In progress"));
        expect(progress.getByTestId("untracked-b-idle")).toBeInTheDocument();
        expect(progress.getByTestId("untracked-b-other")).toBeInTheDocument();
        expect(within(column("Backlog")).queryByTestId("untracked-b-idle")).toBeNull();
        for (const label of ["In Research", "Needs you", "Review", "Merged", "Released"]) {
            expect(within(column(label)).queryByTestId("untracked-b-idle")).toBeNull();
        }
        // The untracked cards come before the tasks.
        const order = Array.from(column("In progress").querySelectorAll("[data-testid]")).map((e) =>
            e.getAttribute("data-testid")
        );
        expect(order.indexOf("untracked-b-idle")).toBeLessThan(order.indexOf("card-4"));
        fireEvent.click(progress.getByTestId("track-b-idle"));
        expect(act).toHaveBeenCalledWith({ action: "track", block_id: "b-idle" });
        fireEvent.click(progress.getByTestId("open-b-other"));
        expect(onOpenAgent).toHaveBeenCalledWith("b-other");
        expect(onOpenAgent).toHaveBeenCalledTimes(1);
    });

    it("gives each card the board's time, agents and colours, and opens the task clicked", () => {
        const colors = { "uid-Worker": "#112233" };
        const t = task(4, "claimed", { holder: working });
        const { onOpen } = mount({ tasks: [t], agents: [working, agent("Idle")], colors });
        const card = screen.getByTestId("card-4");
        expect(card).toHaveAttribute("data-now", String(NOW));
        expect(card).toHaveAttribute("data-agents", "Worker,Idle");
        expect(card).toHaveAttribute("data-colors", JSON.stringify(colors));
        fireEvent.click(card);
        expect(onOpen).toHaveBeenCalledWith(t);
    });

    it("marks the columns with nothing in them, but never the Backlog, and not In progress with untracked agents", () => {
        mount({ tasks: [task(1, "open"), task(6, "blocked")], untracked: [] });
        expect(column("Backlog")).not.toHaveClass("kanban-column--empty");
        expect(column("Needs you")).not.toHaveClass("kanban-column--empty");
        expect(column("In progress")).toHaveClass("kanban-column--empty");
        expect(column("Review")).toHaveClass("kanban-column--empty");
        cleanup();
        mount({ tasks: [], untracked: [working] });
        expect(column("Backlog")).not.toHaveClass("kanban-column--empty");
        expect(column("In progress")).not.toHaveClass("kanban-column--empty");
        expect(column("Needs you")).toHaveClass("kanban-column--empty");
    });
});

describe("KanbanBoard: Review's header", () => {
    const inReview = (n: number, ageMin: number, kind = "") =>
        task(n, "review", { kind, updated_at: NOW - ageMin * MIN });

    it("says how many are waiting, how many are specs, and how long the oldest has waited", () => {
        mount({ tasks: [inReview(1, 5, "spec"), inReview(2, 125), inReview(3, 7, "research")] });
        expect(countText("Review")).toBe("3 · 2 specs · oldest 2h");
    });

    it("says one spec for one", () => {
        mount({ tasks: [inReview(1, 5, "spec"), inReview(2, 30)] });
        expect(countText("Review")).toBe("2 · 1 spec · oldest 30m");
    });

    it("leaves out the specs when none are, and everything but the count when Review is empty", () => {
        mount({ tasks: [inReview(1, 5), inReview(2, 30)] });
        expect(countText("Review")).toBe("2 · oldest 30m");
        cleanup();
        mount({ tasks: [task(8, "done")] });
        expect(countText("Review")).toBe("0");
    });

    it("says nothing of the sort in the other columns", () => {
        mount({ tasks: [inReview(1, 5, "spec"), task(4, "claimed", { holder: working })] });
        expect(countText("In progress")).toBe("1");
        expect(countText("Merged")).toBe("0");
    });

    it("says when the board has more in Review than the limit, and turns the count", () => {
        mount({ tasks: [inReview(1, 5)], reviewLimit: 3, reviewTotal: 4 });
        expect(countText("Review")).toBe("1 · oldest 5m · over 3");
        expect(count("Review")).toHaveClass("kanban-column__count--over");
        expect(count("Review")).toHaveAttribute("data-hover-tip", "More than 3 waiting for review");
        expect(count("Merged")).not.toHaveClass("kanban-column__count--over");
        expect(count("Merged")).not.toHaveAttribute("data-hover-tip");
        // Only Review's header says it.
        expect(countText("Merged")).toBe("0");
        expect(countText("In progress")).toBe("0");
    });

    it("is not over at the limit, or without one", () => {
        mount({ tasks: [inReview(1, 5)], reviewLimit: 3, reviewTotal: 3 });
        expect(countText("Review")).toBe("1 · oldest 5m");
        expect(count("Review")).not.toHaveClass("kanban-column__count--over");
        expect(count("Review")).not.toHaveAttribute("data-hover-tip");
        cleanup();
        mount({ tasks: [inReview(1, 5)], reviewLimit: 0, reviewTotal: 9 });
        expect(countText("Review")).toBe("1 · oldest 5m");
        expect(count("Review")).not.toHaveClass("kanban-column__count--over");
    });

    it("says it is over even when the lane holds none of them", () => {
        mount({ tasks: [], reviewLimit: 2, reviewTotal: 5 });
        expect(countText("Review")).toBe("0 · over 2");
        expect(count("Review")).toHaveClass("kanban-column__count--over");
    });
});

describe("KanbanBoard: cancelled tasks under Merged", () => {
    const tasks = () => [task(8, "done"), task(10, "cancelled"), task(11, "cancelled")];

    it("folds them away and counts only the finished ones in the header", () => {
        mount({ tasks: tasks() });
        expect(cards("Merged")).toEqual(["card-8"]);
        expect(countText("Merged")).toBe("1");
        const fold = within(column("Merged")).getByRole("button", { name: "Cancelled · 2" });
        expect(fold).toHaveAttribute("aria-expanded", "false");
        expect(fold.querySelector("i")).toHaveClass("fa-chevron-right");
    });

    it("opens and folds them again from its button", () => {
        mount({ tasks: tasks() });
        const fold = within(column("Merged")).getByRole("button", { name: "Cancelled · 2" });
        fireEvent.click(fold);
        expect(cards("Merged")).toEqual(["card-8", "card-10", "card-11"]);
        expect(fold).toHaveAttribute("aria-expanded", "true");
        expect(fold.querySelector("i")).toHaveClass("fa-chevron-down");
        fireEvent.click(fold);
        expect(cards("Merged")).toEqual(["card-8"]);
        expect(fold).toHaveAttribute("aria-expanded", "false");
    });

    it("shows the fold button for a single cancelled task, with its class", () => {
        mount({ tasks: [task(10, "cancelled")] });
        const fold = within(column("Merged")).getByRole("button", { name: "Cancelled · 1" });
        expect(fold).toHaveClass("kanban-column__fold");
    });

    it("has no fold button when nothing is cancelled, or in another column", () => {
        mount({ tasks: [task(8, "done")] });
        expect(screen.queryByRole("button", { name: /Cancelled/ })).toBeNull();
    });

    it("is not empty when only cancelled tasks are there", () => {
        mount({ tasks: [task(10, "cancelled")] });
        expect(column("Merged")).not.toHaveClass("kanban-column--empty");
        expect(countText("Merged")).toBe("0");
        expect(column("Released")).toHaveClass("kanban-column--empty");
    });
});

describe("KanbanBoard: the narrow list", () => {
    it("folds Merged and Released until opened, and no other column", () => {
        mount({
            narrow: true,
            tasks: [task(8, "done"), task(9, "done", {}, { pr: { released_in: "v1" } }), task(7, "review")],
        });
        for (const label of ["Merged", "Released"]) {
            const h = head(label);
            expect(h).toHaveAttribute("role", "button");
            expect(h).toHaveAttribute("tabindex", "0");
            expect(h).toHaveAttribute("aria-expanded", "false");
            expect(h).toHaveClass("kanban-column__head--foldable");
            expect(h.querySelector("i")).toHaveClass("fa-chevron-right");
            expect(column(label)).toHaveClass("kanban-column--folded");
        }
        for (const label of ["Backlog", "In Research", "Spec'd", "In progress", "Needs you", "Review"]) {
            const h = head(label);
            expect(h).not.toHaveAttribute("role");
            expect(h).not.toHaveAttribute("tabindex");
            expect(h).not.toHaveAttribute("aria-expanded");
            expect(h).not.toHaveClass("kanban-column__head--foldable");
            expect(h.querySelector("i")).toBeNull();
            expect(column(label)).not.toHaveClass("kanban-column--folded");
        }
    });

    it("opens a folded column from a click on its header, and folds it again", () => {
        mount({ narrow: true, tasks: [task(8, "done")] });
        fireEvent.click(head("Merged"));
        expect(head("Merged")).toHaveAttribute("aria-expanded", "true");
        expect(head("Merged").querySelector("i")).toHaveClass("fa-chevron-down");
        expect(column("Merged")).not.toHaveClass("kanban-column--folded");
        // Released was not touched.
        expect(head("Released")).toHaveAttribute("aria-expanded", "false");
        fireEvent.click(head("Merged"));
        expect(head("Merged")).toHaveAttribute("aria-expanded", "false");
        expect(column("Merged")).toHaveClass("kanban-column--folded");
    });

    it("opens and folds from Enter and Space, and ignores other keys", () => {
        mount({ narrow: true, tasks: [task(8, "done")] });
        fireEvent.keyDown(head("Merged"), { key: "Enter" });
        expect(head("Merged")).toHaveAttribute("aria-expanded", "true");
        fireEvent.keyDown(head("Merged"), { key: " " });
        expect(head("Merged")).toHaveAttribute("aria-expanded", "false");
        fireEvent.keyDown(head("Merged"), { key: "a" });
        expect(head("Merged")).toHaveAttribute("aria-expanded", "false");
        fireEvent.keyDown(head("Merged"), { key: "Escape" });
        expect(head("Merged")).toHaveAttribute("aria-expanded", "false");
        const space = new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true });
        head("Merged").dispatchEvent(space);
        expect(space.defaultPrevented).toBe(true);
    });

    it("a click on a header in the wide layout leaves the narrow list folded", () => {
        const [narrow, setNarrow] = createSignal(false);
        render(() => (
            <KanbanBoard
                tasks={[task(8, "done")]}
                untracked={[]}
                agents={[]}
                model={{ act: vi.fn() } as unknown as KanbanModel}
                now={NOW}
                holding={() => undefined}
                onOpen={() => {}}
                onOpenAgent={() => {}}
                reviewLimit={0}
                reviewTotal={0}
                narrow={narrow()}
            />
        ));
        fireEvent.click(head("Merged"));
        setNarrow(true);
        expect(head("Merged")).toHaveAttribute("aria-expanded", "false");
        expect(column("Merged")).toHaveClass("kanban-column--folded");
    });

    it("does nothing on a click or key in a column that isn't foldable", () => {
        mount({ narrow: true, tasks: [task(7, "review")] });
        fireEvent.click(head("Review"));
        fireEvent.keyDown(head("Review"), { key: "Enter" });
        expect(head("Review")).not.toHaveAttribute("aria-expanded");
        expect(column("Review")).not.toHaveClass("kanban-column--folded");
        const enter = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
        head("Review").dispatchEvent(enter);
        expect(enter.defaultPrevented).toBe(false);
    });

    it("doesn't fold in the wide layout", () => {
        mount({ narrow: false, tasks: [task(8, "done")] });
        expect(head("Merged")).not.toHaveAttribute("role");
        expect(column("Merged")).not.toHaveClass("kanban-column--folded");
        fireEvent.click(head("Merged"));
        expect(column("Merged")).not.toHaveClass("kanban-column--folded");
    });

    it("has no collapse control and no strips, whatever the pane remembers", () => {
        mount({ narrow: true, collapsed: ["review", "backlog"], onToggleCollapsed: vi.fn() });
        expect(screen.queryByRole("button", { name: /^Collapse / })).toBeNull();
        expect(column("Review")).not.toHaveClass("kanban-column--collapsed");
        expect(screen.queryByRole("button", { name: /Expand$/ })).toBeNull();
        expect(document.querySelector(".kanban-board")?.getAttribute("style") ?? "").not.toContain("grid-template");
    });
});

describe("KanbanBoard: collapsing a column", () => {
    it("offers a collapse control in each header when the board can collapse", () => {
        const onToggle = vi.fn();
        mount({ onToggleCollapsed: onToggle });
        const labels = ["Backlog", "In Research", "Spec'd", "In progress", "Needs you", "Review", "Merged", "Released"];
        for (const label of labels) {
            const b = within(head(label)).getByRole("button", { name: `Collapse ${label}` });
            expect(b).toHaveAttribute("aria-expanded", "true");
            expect(b).toHaveClass("kanban-column__collapse");
            expect(b.querySelector("i")).toHaveClass("fa-chevron-left");
        }
        fireEvent.click(within(head("Needs you")).getByRole("button", { name: "Collapse Needs you" }));
        expect(onToggle).toHaveBeenCalledTimes(1);
        expect(onToggle).toHaveBeenCalledWith("needs");
    });

    it("has no collapse control when the board can't collapse", () => {
        mount();
        expect(screen.queryByRole("button", { name: /^Collapse / })).toBeNull();
    });

    it("shows a collapsed column as a strip with its name, count and stats, and a strip takes the place of its header", () => {
        mount({
            tasks: [task(7, "review", { updated_at: NOW - 125 * MIN }), task(6, "review")],
            collapsed: ["review"],
            onToggleCollapsed: vi.fn(),
        });
        const strip = screen.getByRole("button", { name: "Review: 2 tasks, oldest 2h. Expand" });
        expect(strip).toHaveAttribute("title", "Review: 2 tasks, oldest 2h. Click to expand");
        expect(strip).toHaveAttribute("aria-expanded", "false");
        expect(strip).toHaveClass("kanban-collapsed");
        expect(column("Review")).toHaveClass("kanban-column--collapsed");
        expect(strip.querySelector(".kanban-collapsed__chevron")).toHaveClass("fa-chevron-right");
        expect(strip.querySelector(".kanban-collapsed__count")).toHaveTextContent("2");
        expect(strip.querySelector(".kanban-collapsed__name")).toHaveTextContent("Review");
        expect(strip.querySelector(".kanban-collapsed__stat")).toHaveTextContent("oldest 2h");
        expect(column("Review").querySelector(".kanban-column__head")).toBeNull();
        expect(cards("Review")).toEqual([]);
        expect(column("Backlog").querySelector(".kanban-column__head")).not.toBeNull();
    });

    it("says in a strip's words how many are on hold and how long the oldest has waited", () => {
        mount({
            tasks: [task(1, "held", { updated_at: NOW - 3 * MIN }), task(2, "open", { updated_at: NOW - 7 * MIN })],
            collapsed: ["backlog"],
            onToggleCollapsed: vi.fn(),
        });
        const strip = screen.getByRole("button", { name: "Backlog: 2 tasks, 1 on hold, oldest 7m. Expand" });
        const stats = Array.from(strip.querySelectorAll(".kanban-collapsed__stat")).map((e) => e.textContent);
        expect(stats).toEqual(["1 held", "oldest 7m"]);
    });

    it("says 1 task for one, and nothing of hold or age for an empty column", () => {
        mount({ tasks: [task(1, "open")], collapsed: ["backlog", "review"], onToggleCollapsed: vi.fn() });
        expect(screen.getByRole("button", { name: "Backlog: 1 task, oldest 1m. Expand" })).toBeInTheDocument();
        const empty = screen.getByRole("button", { name: "Review: 0 tasks. Expand" });
        expect(empty.querySelector(".kanban-collapsed__stat")).toBeNull();
    });

    it("counts the untracked agents in In progress's strip, and only there", () => {
        mount({
            tasks: [task(4, "claimed", { holder: working })],
            untracked: [agent("A"), agent("B")],
            collapsed: ["progress", "needs"],
            onToggleCollapsed: vi.fn(),
        });
        expect(screen.getByRole("button", { name: /^In progress: 3 tasks,/ })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Needs you: 0 tasks. Expand" })).toBeInTheDocument();
    });

    it("counts the cancelled tasks in Merged's strip", () => {
        mount({
            tasks: [task(8, "done"), task(10, "cancelled")],
            collapsed: ["merged"],
            onToggleCollapsed: vi.fn(),
        });
        expect(screen.getByRole("button", { name: /^Merged: 2 tasks,/ })).toBeInTheDocument();
    });

    it("expands from the strip", () => {
        const onToggle = vi.fn();
        mount({ collapsed: ["released"], onToggleCollapsed: onToggle });
        fireEvent.click(screen.getByRole("button", { name: /^Released: / }));
        expect(onToggle).toHaveBeenCalledTimes(1);
        expect(onToggle).toHaveBeenCalledWith("released");
    });

    it("gives a strip a thin track and the other columns their share, in column order", () => {
        mount({ collapsed: ["research", "merged"], onToggleCollapsed: vi.fn() });
        const style = (document.querySelector(".kanban-board") as HTMLElement).style.getPropertyValue(
            "grid-template-columns"
        );
        expect(style).toBe(
            [
                "minmax(160px, 1fr)",
                "var(--kanban-strip-width)",
                "minmax(160px, 1fr)",
                "minmax(160px, 1fr)",
                "minmax(160px, 1fr)",
                "minmax(160px, 1fr)",
                "var(--kanban-strip-width)",
                "minmax(160px, 1fr)",
            ].join(" ")
        );
    });

    it("sets no tracks when nothing is collapsed", () => {
        mount({ collapsed: [], onToggleCollapsed: vi.fn() });
        expect(
            (document.querySelector(".kanban-board") as HTMLElement).style.getPropertyValue("grid-template-columns")
        ).toBe("");
    });

    it("ignores collapsed columns when the board can't collapse, and names that aren't columns", () => {
        mount({ collapsed: ["review"] });
        expect(column("Review")).not.toHaveClass("kanban-column--collapsed");
        cleanup();
        mount({ collapsed: ["nonsense" as ColumnId], onToggleCollapsed: vi.fn() });
        expect(document.querySelector(".kanban-column--collapsed")).toBeNull();
        expect(
            (document.querySelector(".kanban-board") as HTMLElement).style.getPropertyValue("grid-template-columns")
        ).toBe("");
    });
});

describe("KanbanBoard: Backlog and Spec'd, grouped by the agent each task waits for", () => {
    const waiting = (num: number, who: string, over: Partial<KanbanTask> = {}) =>
        task(num, "open", { target_agent: who, target_agent_uid: who ? `uid-${who}` : "", ...over });

    it("groups the Backlog by agent, alphabetically, with Anyone last, and always offers Anyone", () => {
        mount({
            tasks: [waiting(1, "Zed"), waiting(2, "Amy"), waiting(3, "Amy")],
            agents: [agent("Amy"), agent("Zed")],
        });
        const groups = Array.from(column("Backlog").querySelectorAll(".kanban-group"));
        expect(groups).toHaveLength(3);
        const heads = groups.map((g) =>
            g.querySelector(".kanban-group__head")?.textContent?.replace(/\s+/g, " ").trim()
        );
        expect(heads[0]).toContain("2 waiting");
        expect(heads[1]).toContain("1 waiting");
        expect(heads[2]).toBe("Anyone0 waiting");
        const chips = within(column("Backlog")).getAllByTestId("chip");
        expect(chips.map((c) => c.getAttribute("data-name"))).toEqual(["Amy", "Zed"]);
        expect(groups[2].querySelector(".kanban-group__anyone")).toHaveTextContent("Anyone");
    });

    it("shows Anyone's tasks in its group, and not a second empty Anyone", () => {
        mount({ tasks: [waiting(1, ""), waiting(2, "Amy")], agents: [agent("Amy")] });
        const groups = Array.from(column("Backlog").querySelectorAll(".kanban-group"));
        expect(groups).toHaveLength(2);
        expect(within(groups[1] as HTMLElement).getByTestId("card-1")).toBeInTheDocument();
        expect(groups[1].querySelector(".kanban-group__head")).toHaveTextContent("1 waiting");
    });

    it("gives a group's chip the agent, its first task's agent uid and the colours, and its cards the board's props", () => {
        const colors = { "uid-Amy": "#abcdef" };
        const { onOpen } = mount({ tasks: [waiting(1, "Amy")], agents: [agent("Amy"), agent("Zed")], colors });
        const chip = screen.getByTestId("chip");
        expect(chip).toHaveAttribute("data-name", "Amy");
        expect(chip).toHaveAttribute("data-uid", "uid-Amy");
        expect(chip).toHaveAttribute("data-agent", "Amy");
        expect(chip).toHaveAttribute("data-colors", JSON.stringify(colors));
        const card = screen.getByTestId("card-1");
        expect(card).toHaveAttribute("data-now", String(NOW));
        expect(card).toHaveAttribute("data-agents", "Amy,Zed");
        expect(card).toHaveAttribute("data-colors", JSON.stringify(colors));
        fireEvent.click(card);
        expect(onOpen).toHaveBeenCalledTimes(1);
        expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: "w1" }));
    });

    it("says the agent's state under its group, or what it is busy on", () => {
        const amy = agent("Amy", { state: "working" });
        mount({ tasks: [waiting(1, "Amy")], agents: [amy] });
        expect(column("Backlog").querySelector(".warden-manager-dim")).toHaveTextContent("1 waiting, state:working");
        cleanup();
        const busy = task(7, "claimed", { holder: amy });
        mount({ tasks: [waiting(1, "Amy")], agents: [amy], holding: (a) => (a === "Amy" ? busy : undefined) });
        const dim = column("Backlog").querySelector(".warden-manager-dim") as HTMLElement;
        expect(dim).toHaveTextContent("1 waiting, busy on #7");
        expect(dim).not.toHaveTextContent("state:");
    });

    it("says nothing of state for a group whose agent isn't known, or for Anyone", () => {
        mount({ tasks: [waiting(1, "Ghost"), waiting(2, "")], agents: [] });
        const dims = Array.from(column("Backlog").querySelectorAll(".warden-manager-dim")).map((e) =>
            (e.textContent ?? "").replace(/\s+/g, " ").trim()
        );
        expect(dims).toEqual(["1 waiting", "1 waiting"]);
    });

    it("never asks what Anyone is busy on", () => {
        const holding = vi.fn(() => undefined);
        mount({ tasks: [waiting(1, "")], holding });
        expect(holding).not.toHaveBeenCalledWith("");
    });

    it("groups Spec'd by agent too, but shows only the groups it has", () => {
        mount({
            tasks: [
                waiting(1, "Amy", {}),
                task(2, "open", { target_agent: "Amy", target_agent_uid: "uid-Amy" }, { after: [spec] }),
            ],
            agents: [agent("Amy")],
        });
        const specd = column("Spec'd");
        expect(specd.querySelectorAll(".kanban-group")).toHaveLength(1);
        expect(within(specd).getByTestId("card-2")).toBeInTheDocument();
        cleanup();
        mount({ tasks: [], agents: [] });
        expect(column("Spec'd").querySelectorAll(".kanban-group")).toHaveLength(0);
        expect(column("Backlog").querySelectorAll(".kanban-group")).toHaveLength(1);
    });

    it("keeps a group mounted, and its drop target registered, while its tasks change", () => {
        const amy = agent("Amy");
        const [tasks, setTasks] = createSignal<KanbanTask[]>([waiting(1, "Amy")]);
        render(() => (
            <KanbanBoard
                tasks={tasks()}
                untracked={[]}
                agents={[amy]}
                holding={() => undefined}
                reviewLimit={0}
                reviewTotal={0}
                model={{ act: vi.fn() } as unknown as KanbanModel}
                now={NOW}
                onOpen={vi.fn()}
                onOpenAgent={vi.fn()}
            />
        ));
        const groups = () => Array.from(column("Backlog").querySelectorAll(".kanban-group"));
        const before = groups();
        const registered = [...targets];
        expect(before).toHaveLength(2);
        setTasks([waiting(1, "Amy"), waiting(2, "Amy")]);
        expect(within(column("Backlog")).getByTestId("card-2")).toBeInTheDocument();
        expect(groups()).toEqual(before);
        expect(groups().every((g, i) => g === before[i])).toBe(true);
        expect(targets.length).toBe(registered.length);
        expect(targets.every((t, i) => t === registered[i])).toBe(true);
        setTasks([]);
        expect(groups()).toHaveLength(1);
        expect(groups()[0]).toBe(before[1]);
        expect(before[0].isConnected).toBe(false);
    });
});

describe("KanbanBoard: dropping a card", () => {
    const waiting = (num: number, who: string, rank?: number) =>
        task(num, "open", { target_agent: who, target_agent_uid: who ? `uid-${who}` : "" }, { rank });

    it("gives the dropped task to the group's agent, at the end of its backlog", () => {
        vi.useFakeTimers();
        const { act } = mount({ tasks: [waiting(1, "Amy", 1000), waiting(2, "Amy", 3000)], agents: [agent("Amy")] });
        const group = column("Backlog").querySelector(".kanban-group") as HTMLElement;
        const target = dropAt(group);
        target.onDrop(dragged("w9"));
        // After pragmatic-dnd has finished dispatching.
        expect(act).not.toHaveBeenCalled();
        vi.runAllTimers();
        expect(act).toHaveBeenCalledTimes(1);
        expect(act).toHaveBeenCalledWith({ action: "move", id: "w9", agent: "Amy", rank: 4000 });
    });

    it("ranks a drop on an empty Anyone by the clock, and gives the task to anyone", () => {
        vi.useFakeTimers();
        vi.spyOn(Date, "now").mockReturnValue(777_000);
        const { act } = mount({ tasks: [], agents: [] });
        const group = column("Backlog").querySelector(".kanban-group") as HTMLElement;
        dropAt(group).onDrop(dragged("w9"));
        vi.runAllTimers();
        expect(act).toHaveBeenCalledWith({ action: "move", id: "w9", agent: "", rank: 777_000 });
    });

    it("ranks by a task's filing time when it has no rank of its own", () => {
        vi.useFakeTimers();
        const t = waiting(1, "Amy");
        const { act } = mount({ tasks: [t], agents: [agent("Amy")] });
        dropAt(column("Backlog").querySelector(".kanban-group") as HTMLElement).onDrop(dragged("w9"));
        vi.runAllTimers();
        expect(act).toHaveBeenCalledWith({ action: "move", id: "w9", agent: "Amy", rank: t.created_at + 1000 });
    });

    it("takes only Kanban cards, and ignores a drop that carries no id", () => {
        vi.useFakeTimers();
        const { act } = mount({ tasks: [], agents: [] });
        const target = dropAt(column("Backlog").querySelector(".kanban-group") as HTMLElement);
        expect(target.canDrop(dragged("w9"))).toBe(true);
        expect(target.canDrop({ source: { data: { type: "something-else", id: "w9" } } })).toBe(false);
        target.onDrop({ source: { data: { type: kanbanCardItemType, id: 5 } } });
        vi.runAllTimers();
        expect(act).not.toHaveBeenCalled();
    });

    it("lights a group while a card is over it, and not after it leaves or drops", () => {
        vi.useFakeTimers();
        mount({ tasks: [], agents: [] });
        const group = column("Backlog").querySelector(".kanban-group") as HTMLElement;
        const target = dropAt(group);
        expect(group).not.toHaveClass("kanban-drop--hover");
        target.onDragEnter();
        expect(group).toHaveClass("kanban-drop--hover");
        target.onDragLeave();
        expect(group).not.toHaveClass("kanban-drop--hover");
        target.onDragEnter();
        target.onDrop(dragged("w9"));
        expect(group).not.toHaveClass("kanban-drop--hover");
        vi.runAllTimers();
    });

    it("lets a card be dropped on a Spec'd group too", () => {
        vi.useFakeTimers();
        const t = task(2, "open", { target_agent: "Amy", target_agent_uid: "uid-Amy" }, { after: [spec], rank: 500 });
        const { act } = mount({ tasks: [t], agents: [agent("Amy")] });
        dropAt(column("Spec'd").querySelector(".kanban-group") as HTMLElement).onDrop(dragged("w9"));
        vi.runAllTimers();
        expect(act).toHaveBeenCalledWith({ action: "move", id: "w9", agent: "Amy", rank: 1500 });
    });

    it("takes a drop on the strip of a collapsed waiting column, to anyone, at the end of the column", () => {
        vi.useFakeTimers();
        const { act } = mount({
            tasks: [waiting(1, "Amy", 100), waiting(2, "Zed", 900)],
            agents: [],
            collapsed: ["backlog"],
            onToggleCollapsed: vi.fn(),
        });
        const strip = screen.getByRole("button", { name: /^Backlog: / });
        const target = dropAt(strip);
        expect(target.canDrop(dragged("w9"))).toBe(true);
        target.onDrop(dragged("w9"));
        vi.runAllTimers();
        expect(act).toHaveBeenCalledWith({ action: "move", id: "w9", agent: "", rank: 1900 });
    });

    it("takes a drop on an empty collapsed Spec'd strip by the clock", () => {
        vi.useFakeTimers();
        vi.spyOn(Date, "now").mockReturnValue(555_000);
        const { act } = mount({ collapsed: ["specd"], onToggleCollapsed: vi.fn() });
        dropAt(screen.getByRole("button", { name: /^Spec'd: / })).onDrop(dragged("w9"));
        vi.runAllTimers();
        expect(act).toHaveBeenCalledWith({ action: "move", id: "w9", agent: "", rank: 555_000 });
    });

    it("lights a strip while a card is over it", () => {
        mount({ collapsed: ["backlog"], onToggleCollapsed: vi.fn() });
        const strip = screen.getByRole("button", { name: /^Backlog: / });
        expect(strip).toHaveClass("kanban-collapsed");
        expect(strip).not.toHaveClass("kanban-drop--hover");
        dropAt(strip).onDragEnter();
        expect(strip).toHaveClass("kanban-collapsed", "kanban-drop--hover");
        dropAt(strip).onDragLeave();
        expect(strip).not.toHaveClass("kanban-drop--hover");
    });

    it("refuses a card on the strip of a column that isn't waiting", () => {
        mount({
            collapsed: ["progress", "review", "needs", "merged", "research", "released"],
            onToggleCollapsed: vi.fn(),
        });
        for (const label of ["In progress", "Review", "Needs you", "Merged", "In Research", "Released"]) {
            const strip = within(column(label)).getByRole("button", { name: new RegExp(`^${label}: `) });
            expect(dropAt(strip).canDrop(dragged("w9"))).toBe(false);
        }
    });
});
