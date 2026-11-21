// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The narrow Kanban's bar of section links and Collapse all (kanban-section-bar.tsx): one link per section in the order
// the list draws them, with the board's own stage names, counts and colours; a link opens its section and brings it to
// the top; Collapse all and Expand all set the same fold state a section's header does; the wide layout has no bar.

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./kanban-card", () => ({
    AgentChip: () => null,
    KanbanCard: () => null,
    UntrackedCard: () => null,
    agentStateWords: () => "",
}));

import { KanbanBoard } from "./kanban-board";
import type { KanbanModel } from "./kanban-model";
import { currentSection, sectionLinkLabel, sectionOrder } from "./kanban-section-bar";
import { defaultStages, type KanbanStage } from "./kanban-stages";
import type { KanbanTask } from "./kanban-types";

function task(num: number, state: KanbanTask["state"], updated_at = 1): KanbanTask {
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

// Two in Review, one done (Merged), one on hold (Backlog), one waiting on the operator (Needs you).
const tasks = [task(1, "review"), task(2, "review"), task(3, "done"), task(4, "held"), task(5, "blocked")];

function board(over: { narrow?: boolean; sectionBar?: boolean; stages?: readonly KanbanStage[] } = {}) {
    return render(() => (
        <KanbanBoard
            tasks={tasks}
            untracked={[]}
            agents={[]}
            model={{ act: vi.fn() } as unknown as KanbanModel}
            now={10_000_000}
            holding={() => undefined}
            onOpen={() => {}}
            onOpenAgent={() => {}}
            reviewLimit={0}
            reviewTotal={2}
            narrow={over.narrow ?? true}
            sectionBar={over.sectionBar ?? true}
            stages={over.stages}
        />
    ));
}

const bar = () => document.querySelector(".kanban-section-bar") as HTMLElement | null;
const links = () => Array.from(document.querySelectorAll<HTMLAnchorElement>(".kanban-section-bar__link"));
const link = (name: string) => links().find((a) => a.querySelector(".kanban-section-bar__name")?.textContent === name)!;
const column = (label: string) => screen.getByRole("region", { name: label });
const head = (label: string) => column(label).querySelector(".kanban-column__head") as HTMLElement;
const allButton = () => screen.getByRole("button", { name: /^(Collapse|Expand) all$/ });

afterEach(cleanup);

describe("the section bar's helpers", () => {
    it("orders the sections as the list draws them: the stage that shows what needs you first, the rest in the board's order", () => {
        const order = sectionOrder(defaultStages()).map((s) => s.name);
        expect(order[0]).toBe("Needs you");
        expect(order.slice(1)).toEqual(defaultStages().filter((s) => s.name !== "Needs you").map((s) => s.name));
    });

    it("says each link for a screen reader, with its count", () => {
        expect(sectionLinkLabel({ name: "Merged", count: 3 })).toBe("go to Merged, 3 tasks");
        expect(sectionLinkLabel({ name: "Merged", count: 1 })).toBe("go to Merged, 1 task");
        expect(sectionLinkLabel({ name: "Backlog", count: 0 })).toBe("go to Backlog, 0 tasks");
    });

    it("finds the section you are in: the last whose top has reached the line under the bar", () => {
        const tops = [0, 200, 500, 900];
        expect(currentSection(tops, 0, 40)).toBe(0);
        expect(currentSection(tops, 150, 40)).toBe(0);
        expect(currentSection(tops, 170, 40)).toBe(1);
        expect(currentSection(tops, 460, 40)).toBe(2);
        expect(currentSection(tops, 5000, 40)).toBe(3);
        expect(currentSection([], 0, 0)).toBe(0);
    });
});

describe("the bar in the narrow list", () => {
    it("has one link per section, Needs you first as the list draws it, with its count and stage colour", () => {
        board();
        expect(bar()).not.toBeNull();
        expect(links().map((a) => a.querySelector(".kanban-section-bar__name")!.textContent)).toEqual([
            "Needs you",
            "Backlog",
            "In Research",
            "Spec'd",
            "In progress",
            "Review",
            "Merged",
            "Released",
        ]);
        expect(link("Review").querySelector(".kanban-section-bar__count")!.textContent).toBe("2");
        expect(link("Merged").querySelector(".kanban-section-bar__count")!.textContent).toBe("1");
        expect(link("Released").querySelector(".kanban-section-bar__count")!.textContent).toBe("0");
        // The mark takes its colour from the stage's own name, as the section does.
        expect(link("Review").getAttribute("data-stage-colour")).toBe(column("Review").getAttribute("data-stage-colour"));
    });

    it("follows the board's stage settings: a renamed stage has its own name, and a board's own set is the bar's", () => {
        const stages = defaultStages().map((s) => (s.name === "Review" ? { ...s, name: "Ready to merge" } : s));
        board({ stages });
        expect(link("Ready to merge")).toBeDefined();
        expect(links().some((a) => a.textContent?.includes("Review"))).toBe(false);
        expect(link("Ready to merge").getAttribute("aria-label")).toBe("go to Ready to merge, 2 tasks");
        // A board with only two stages has only two links.
        board({ stages: defaultStages().slice(0, 2) });
        expect(document.querySelectorAll(".kanban-section-bar").length).toBe(2);
    });

    it("makes real links and a real button: reachable in order, with the words a screen reader hears", () => {
        board();
        const all = links();
        expect(all.every((a) => a.tagName === "A" && a.getAttribute("href")?.startsWith("#kanban-section-"))).toBe(true);
        expect(all[0].getAttribute("aria-label")).toBe("go to Needs you, 1 task");
        // No tabindex that would reorder them; the bar is a labelled navigation, and Collapse all comes after the links.
        expect(all.every((a) => !a.hasAttribute("tabindex"))).toBe(true);
        expect(bar()!.tagName).toBe("NAV");
        expect(bar()!.getAttribute("aria-label")).toBe("Sections");
        expect(allButton().tagName).toBe("BUTTON");
        expect(bar()!.lastElementChild).toBe(allButton());
    });

    it("is not drawn in the wide layout, nor when the list has no bar (a lane)", () => {
        board({ narrow: false });
        expect(bar()).toBeNull();
        cleanup();
        board({ narrow: true, sectionBar: false });
        expect(bar()).toBeNull();
        // And it goes when the layout turns wide.
        cleanup();
        const [narrow, setNarrow] = createSignal(true);
        render(() => (
            <KanbanBoard
                tasks={tasks}
                untracked={[]}
                agents={[]}
                model={{ act: vi.fn() } as unknown as KanbanModel}
                now={10_000_000}
                holding={() => undefined}
                onOpen={() => {}}
                onOpenAgent={() => {}}
                reviewLimit={0}
                reviewTotal={2}
                narrow={narrow()}
                sectionBar
            />
        ));
        expect(bar()).not.toBeNull();
        setNarrow(false);
        expect(bar()).toBeNull();
    });
});

describe("a link", () => {
    it("opens its section if it is folded and brings its header to the top", async () => {
        const scrollIntoView = vi.fn();
        const original = Element.prototype.scrollIntoView;
        Element.prototype.scrollIntoView = scrollIntoView;
        try {
            board();
            expect(column("Merged")).toHaveClass("kanban-column--folded");
            const click = new MouseEvent("click", { bubbles: true, cancelable: true });
            link("Merged").dispatchEvent(click);
            // A link, not a navigation: the page's address is left alone.
            expect(click.defaultPrevented).toBe(true);
            expect(column("Merged")).not.toHaveClass("kanban-column--folded");
            expect(head("Merged")).toHaveAttribute("aria-expanded", "true");
            await Promise.resolve();
            expect(scrollIntoView).toHaveBeenCalledTimes(1);
            expect(scrollIntoView.mock.contexts[0]).toBe(column("Merged"));
            expect(scrollIntoView).toHaveBeenCalledWith({ block: "start" });
        } finally {
            Element.prototype.scrollIntoView = original;
        }
    });

    it("leaves a section that is open open, and does not fold one that was opened by its header", async () => {
        Element.prototype.scrollIntoView = vi.fn();
        board();
        fireEvent.click(link("Review"));
        expect(column("Review")).not.toHaveClass("kanban-column--folded");
        fireEvent.click(head("Merged"));
        fireEvent.click(link("Merged"));
        expect(column("Merged")).not.toHaveClass("kanban-column--folded");
    });
});

describe("Collapse all", () => {
    it("folds every section, then reads Expand all and opens every one", () => {
        board();
        expect(allButton()).toHaveTextContent("Collapse all");
        fireEvent.click(allButton());
        for (const stage of defaultStages()) {
            expect(column(stage.name), stage.name).toHaveClass("kanban-column--folded");
            expect(head(stage.name)).toHaveAttribute("aria-expanded", "false");
        }
        expect(allButton()).toHaveTextContent("Expand all");
        fireEvent.click(allButton());
        for (const stage of defaultStages()) {
            expect(column(stage.name), stage.name).not.toHaveClass("kanban-column--folded");
        }
        expect(allButton()).toHaveTextContent("Collapse all");
    });

    it("sets the state a section's own header sets: one opened by hand takes the button back to Collapse all", () => {
        board();
        fireEvent.click(allButton());
        expect(allButton()).toHaveTextContent("Expand all");
        fireEvent.click(head("Review"));
        expect(column("Review")).not.toHaveClass("kanban-column--folded");
        expect(allButton()).toHaveTextContent("Collapse all");
        // And folding the last open one by its header makes it Expand all again.
        fireEvent.click(head("Review"));
        expect(allButton()).toHaveTextContent("Expand all");
    });

    it("starts as Collapse all, because Merged and Released start folded and the rest do not", () => {
        board();
        expect(allButton()).toHaveTextContent("Collapse all");
    });
});

describe("the section you are in", () => {
    it("is marked as current while the list scrolls, by its link's outline and aria-current", () => {
        board();
        const list = document.querySelector(".kanban-board") as HTMLElement;
        // The sections' tops, in the order the list draws them: Needs you, Backlog, In Research, Spec'd, In progress, Review...
        const order = ["needs", "backlog", "research", "specd", "progress", "review", "merged", "released"];
        const names = ["Needs you", "Backlog", "In Research", "Spec'd", "In progress", "Review", "Merged", "Released"];
        names.forEach((name, i) => Object.defineProperty(column(name), "offsetTop", { configurable: true, value: i * 300 }));
        expect(order.length).toBe(names.length);
        list.scrollTop = 0;
        fireEvent.scroll(list);
        expect(link("Needs you")).toHaveAttribute("aria-current", "location");
        expect(link("Needs you")).toHaveClass("kanban-section-bar__link--current");
        list.scrollTop = 1000;
        fireEvent.scroll(list);
        expect(link("Needs you")).not.toHaveAttribute("aria-current");
        expect(link("Spec'd")).toHaveAttribute("aria-current", "location");
        expect(document.querySelectorAll('[aria-current="location"]')).toHaveLength(1);
        list.scrollTop = 1650;
        fireEvent.scroll(list);
        expect(link("Review")).toHaveAttribute("aria-current", "location");
    });
    describe("a link that cannot bring its section to the top", () => {
        const names = ["Needs you", "Backlog", "In Research", "Spec'd", "In progress", "Review", "Merged", "Released"];
        const atEnd = () => {
            board();
            const list = document.querySelector(".kanban-board") as HTMLElement;
            names.forEach((name, i) => Object.defineProperty(column(name), "offsetTop", { configurable: true, value: i * 300 }));
            // The list is at its end and Released is short: its top (2100) is further down than the list can scroll (1700).
            list.scrollTop = 1700;
            fireEvent.scroll(list);
            expect(link("Review")).toHaveAttribute("aria-current", "location");
            return list;
        };
        const press = async (name: string) => {
            const original = Element.prototype.scrollIntoView;
            Element.prototype.scrollIntoView = vi.fn();
            try {
                link(name).dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
                await Promise.resolve();
            } finally {
                Element.prototype.scrollIntoView = original;
            }
        };

        it("still marks that section as the one you are in", async () => {
            atEnd();
            await press("Released");
            expect(link("Released")).toHaveAttribute("aria-current", "location");
            expect(document.querySelectorAll('[aria-current="location"]')).toHaveLength(1);
        });

        it("hands the mark back when the person scrolls the list themselves", async () => {
            const list = atEnd();
            await press("Released");
            fireEvent.wheel(list);
            expect(link("Review")).toHaveAttribute("aria-current", "location");
        });

        it("lets go once the scroll has brought the section to the top, and does not come back", async () => {
            const list = atEnd();
            await press("Released");
            list.scrollTop = 2100;
            fireEvent.scroll(list);
            expect(link("Released")).toHaveAttribute("aria-current", "location");
            list.scrollTop = 1000;
            fireEvent.scroll(list);
            expect(link("Spec'd")).toHaveAttribute("aria-current", "location");
        });
    });
});
