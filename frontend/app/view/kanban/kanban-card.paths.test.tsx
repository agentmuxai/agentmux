// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Kanban card (KanbanCard, AgentChip and UntrackedCard in kanban-card.tsx), path by path: which tasks
// can be dragged, whose chip and colour a card shows, the reason or note line, every pill and what it says
// and where, the foot, how a click or key opens it, and the dashed card of an agent with no task. The
// drag adapter, the media thumbnails, the colours, the seats and the block store are stand-ins.

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { drags, dragCleanup, colorOf, seatOfFn, blocks } = vi.hoisted(() => ({
    drags: [] as { element: HTMLElement; getInitialData: () => unknown }[],
    dragCleanup: vi.fn(),
    colorOf: vi.fn(),
    seatOfFn: vi.fn(),
    blocks: {} as Record<string, { meta?: Record<string, unknown> } | undefined>,
}));

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    draggable: (cfg: (typeof drags)[number]) => {
        drags.push(cfg);
        return dragCleanup;
    },
}));
vi.mock("./kanban-media", () => ({
    CardMedia: (props: { task: { id: string } }) => <div data-testid="media" data-task={props.task.id} />,
}));
vi.mock("./kanban-colors", () => ({
    agentColor: (...args: unknown[]) => colorOf(...args),
}));
vi.mock("./kanban-seats", () => ({
    useSeatIndex: () => () => seatIndex,
    seatOf: (...args: unknown[]) => seatOfFn(...args),
}));
vi.mock("@/app/agent-model/seat-format", () => ({
    seatTooltipLine: (seat: { name: string }) => `seat:${seat.name}`,
}));
vi.mock("@/app/store/global", () => ({
    MOS: { getMuxObjectAtom: (id: string) => () => blocks[id.replace(/^block:/, "")] },
}));

import { kanbanCardItemType } from "@/app/drag/drag-types";
import { AgentChip, agentStateWords, agentTone, KanbanCard, UntrackedCard } from "./kanban-card";
import type { KanbanAgent, KanbanAgentState, KanbanCardFields, KanbanTask } from "./kanban-types";

const seatIndex = { byUid: new Map(), uidByName: new Map() };
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

const link = (num: number, state: KanbanTask["state"], kind?: string, title = `link ${num}`) => ({
    id: `l${num}`,
    num,
    title,
    state,
    kind,
});

interface Mounted {
    container: HTMLElement;
    card: HTMLElement;
    onOpen: ReturnType<typeof vi.fn>;
    unmount: () => void;
}

function mount(
    t: KanbanTask,
    over: { agents?: KanbanAgent[]; colors?: Record<string, string>; now?: number } = {}
): Mounted {
    const onOpen = vi.fn();
    const props = { task: t, now: over.now ?? NOW, agents: over.agents ?? [], colors: over.colors, onOpen };
    const result = render(() => <KanbanCard {...props} />);
    return { ...result, card: result.container.querySelector(".kanban-card") as HTMLElement, onOpen };
}

const pills = (root: HTMLElement) =>
    [...root.querySelectorAll(".ui-pill")].map((p) => ({
        text: p.textContent,
        icon: p.querySelector("i")?.className.replace("fa-solid fa-", "") ?? null,
        tone: /ui-pill--(\w+)/.exec(p.className)?.[1],
        title: p.getAttribute("title"),
    }));
const pillTexts = (root: HTMLElement) => pills(root).map((p) => p.text);
const pillWith = (root: HTMLElement, text: RegExp | string) =>
    pills(root).find((p) => (typeof text === "string" ? p.text === text : text.test(p.text ?? "")));

beforeEach(() => {
    drags.length = 0;
    dragCleanup.mockClear();
    colorOf.mockReset();
    colorOf.mockReturnValue(undefined);
    seatOfFn.mockReset();
    seatOfFn.mockReturnValue(null);
    for (const k of Object.keys(blocks)) delete blocks[k];
});
afterEach(() => cleanup());

describe("agent state words and tones", () => {
    it("words every state, and gives each its tone", () => {
        const table: [KanbanAgentState, string, string][] = [
            ["working", "working", "busy"],
            ["waiting", "waiting on a question", "warn"],
            ["idle", "idle", "ok"],
            ["stopped", "stopped", "idle"],
            ["error", "errored", "error"],
            ["limited", "out of quota", "warn"],
            ["signed_out", "signed out", "error"],
            ["gone", "closed", "idle"],
        ];
        for (const [state, words, tone] of table) {
            expect([state, agentStateWords(state), agentTone(state)]).toEqual([state, words, tone]);
        }
        expect(agentStateWords(undefined)).toBe("not here");
        expect(agentTone(undefined)).toBe("idle");
    });
});

describe("KanbanCard: the card", () => {
    it("is a button named for its task, with its number and title", () => {
        const { card } = mount(task(5, "open"));
        expect(card.getAttribute("role")).toBe("button");
        expect(card.getAttribute("tabindex")).toBe("0");
        expect(card.getAttribute("aria-label")).toBe("Task 5: task 5");
        expect(card.querySelector(".kanban-card__num")).toHaveTextContent(/^#5$/);
        expect(card.querySelector(".kanban-card__text")).toHaveTextContent(/^task 5$/);
        expect(card.className).toBe("kanban-card");
    });

    it("marks a held task, a task that needs you, and a cancelled one", () => {
        expect(mount(task(1, "held")).card).toHaveClass("kanban-card--held");
        cleanup();
        expect(mount(task(2, "blocked")).card).toHaveClass("kanban-card--needs");
        cleanup();
        expect(mount(task(3, "cancelled")).card).toHaveClass("kanban-card--cancelled");
        cleanup();
        const plain = mount(task(4, "open")).card;
        expect(plain).not.toHaveClass("kanban-card--held");
        expect(plain).not.toHaveClass("kanban-card--needs");
        expect(plain).not.toHaveClass("kanban-card--cancelled");
    });

    it("opens its task on a click", () => {
        const t = task(5, "open");
        const { card, onOpen } = mount(t);
        fireEvent.click(card);
        expect(onOpen).toHaveBeenCalledTimes(1);
        expect(onOpen).toHaveBeenCalledWith(t);
    });

    it("opens on Enter and on Space, without scrolling, and leaves other keys alone", () => {
        const t = task(5, "open");
        const { card, onOpen } = mount(t);
        expect(fireEvent.keyDown(card, { key: "Enter" })).toBe(false);
        expect(fireEvent.keyDown(card, { key: " " })).toBe(false);
        expect(onOpen).toHaveBeenCalledTimes(2);
        expect(onOpen).toHaveBeenCalledWith(t);
        expect(fireEvent.keyDown(card, { key: "Tab" })).toBe(true);
        expect(fireEvent.keyDown(card, { key: "a" })).toBe(true);
        expect(onOpen).toHaveBeenCalledTimes(2);
    });

    it("shows the task's media", () => {
        const { container } = mount(task(5, "open"));
        expect(container.querySelector('[data-testid="media"]')?.getAttribute("data-task")).toBe("w5");
    });
});

describe("KanbanCard: dragging", () => {
    it("lets a waiting task be dragged, with its id and number", () => {
        const { card } = mount(task(5, "open"));
        expect(drags).toHaveLength(1);
        expect(drags[0].element).toBe(card);
        expect(drags[0].getInitialData()).toEqual({ type: kanbanCardItemType, id: "w5", num: 5 });
    });

    it("lets a held task, and a Spec'd one, be dragged too", () => {
        mount(task(1, "held"));
        const spec = { after: [link(9, "done", "spec")] };
        mount(task(2, "open", {}, spec));
        expect(drags).toHaveLength(2);
    });

    it("lets no other task be dragged", () => {
        for (const t of [
            task(1, "claimed", { claimed_by: "Amy", holder: agent("Amy") }),
            task(2, "review"),
            task(3, "blocked"),
            task(4, "failed"),
            task(5, "done"),
            task(6, "cancelled"),
        ]) {
            mount(t);
            cleanup();
        }
        expect(drags).toHaveLength(0);
    });

    it("stops being draggable when the card goes", () => {
        const { unmount } = mount(task(5, "open"));
        expect(dragCleanup).not.toHaveBeenCalled();
        unmount();
        expect(dragCleanup).toHaveBeenCalledTimes(1);
    });
});

describe("KanbanCard: whose it is", () => {
    it("shows the agent holding it, in the state it has, with its colour on the card's edge", () => {
        const amy = agent("Amy", { state: "working" });
        colorOf.mockImplementation((a, _uid, _colors, role) => (role === "identity" ? "#identity" : "#edge"));
        const { card, container } = mount(
            task(5, "claimed", { claimed_by: "Amy", claimed_by_uid: "uid-Amy", holder: amy }),
            {
                colors: { x: "y" },
            }
        );
        const chip = container.querySelector(".kanban-agent-chip") as HTMLElement;
        expect(chip.querySelector(".kanban-agent-chip__name")).toHaveTextContent(/^Amy$/);
        const dot = chip.querySelector(".ui-status-dot") as HTMLElement;
        expect(dot).toHaveClass("ui-status-dot--busy");
        expect(dot.getAttribute("aria-label")).toBe("Amy: working");
        expect(card.style.getPropertyValue("--kanban-agent-color")).toBe("#edge");
        expect(
            (chip.querySelector(".kanban-agent-chip__name") as HTMLElement).style.getPropertyValue(
                "--kanban-agent-text"
            )
        ).toBe("#identity");
        expect(colorOf).toHaveBeenCalledWith(amy, "uid-Amy", { x: "y" });
        expect(colorOf).toHaveBeenCalledWith(amy, "uid-Amy", { x: "y" }, "identity");
    });

    it("shows the holder rather than the agent it was meant for", () => {
        const holder = agent("Bea", { state: "idle" });
        const { container } = mount(
            task(5, "claimed", {
                claimed_by: "Bea",
                claimed_by_uid: "uid-Bea",
                holder,
                target_agent: "Amy",
                target_agent_uid: "uid-Amy",
            }),
            { agents: [agent("Amy")] }
        );
        expect(container.querySelector(".kanban-agent-chip__name")).toHaveTextContent(/^Bea$/);
    });

    it("shows the agent a waiting task is for, as the board knows it, or as not here", () => {
        const amy = agent("Amy", { state: "limited" });
        const { container } = mount(task(5, "open", { target_agent: "Amy", target_agent_uid: "uid-Amy" }), {
            agents: [agent("Zed"), amy],
        });
        expect(container.querySelector(".ui-status-dot")?.getAttribute("aria-label")).toBe("Amy: out of quota");
        expect(colorOf).toHaveBeenCalledWith(amy, "uid-Amy", undefined);
        cleanup();
        const ghost = mount(task(6, "open", { target_agent: "Ghost", target_agent_uid: "uid-Ghost" }), {
            agents: [amy],
        });
        expect(ghost.container.querySelector(".ui-status-dot")?.getAttribute("aria-label")).toBe("Ghost: not here");
        expect(ghost.container.querySelector(".ui-status-dot")).toHaveClass("ui-status-dot--idle");
        expect(colorOf).toHaveBeenCalledWith(null, "uid-Ghost", undefined);
    });

    it("shows no chip, and no colour, for a task that is for no one", () => {
        const { card, container } = mount(task(5, "open"));
        expect(container.querySelector(".kanban-agent-chip")).toBeNull();
        expect(card.getAttribute("style")).toBeNull();
    });

    it("keeps the plain border when the agent has no colour", () => {
        const { card } = mount(task(5, "open", { target_agent: "Amy", target_agent_uid: "uid-Amy" }), {
            agents: [agent("Amy")],
        });
        expect(card.style.getPropertyValue("--kanban-agent-color")).toBe("");
    });

    it("shows the owner's seat as a tooltip, by uid, or by the agent's own uid, or by name", () => {
        seatOfFn.mockReturnValue({ name: "lead" });
        const holder = agent("Amy");
        const { container } = mount(task(5, "claimed", { claimed_by: "Amy", claimed_by_uid: "uid-task", holder }));
        const chip = container.querySelector(".kanban-agent-chip") as HTMLElement;
        expect(chip.getAttribute("data-hover-tip")).toBe("Amy: seat:lead");
        expect(seatOfFn).toHaveBeenCalledWith(seatIndex, "uid-task", "Amy");
        cleanup();
        seatOfFn.mockClear();
        mount(task(6, "claimed", { claimed_by: "Amy", claimed_by_uid: "", holder }));
        expect(seatOfFn).toHaveBeenCalledWith(seatIndex, "uid-Amy", "Amy");
    });

    it("asks for the seat of the name the card shows, not of the agent's own name", () => {
        const holder = agent("Amy-pane");
        mount(task(5, "claimed", { claimed_by: "Amy", claimed_by_uid: "", holder }));
        expect(seatOfFn).toHaveBeenCalledWith(seatIndex, "uid-Amy-pane", "Amy");
    });

    it("shows no seat tooltip when the owner has no seat", () => {
        const { container } = mount(
            task(5, "claimed", { claimed_by: "Amy", claimed_by_uid: "u", holder: agent("Amy") })
        );
        expect(container.querySelector(".kanban-agent-chip")?.hasAttribute("data-hover-tip")).toBe(false);
    });
});

describe("AgentChip", () => {
    it("shows an agent by its name and state, with no seat unless asked", () => {
        seatOfFn.mockReturnValue({ name: "lead" });
        const { container } = render(() => <AgentChip name="Amy" agent={agent("Amy", { state: "error" })} />);
        expect(container.querySelector(".ui-status-dot")?.getAttribute("aria-label")).toBe("Amy: errored");
        expect(container.querySelector(".ui-status-dot")).toHaveClass("ui-status-dot--error");
        expect(container.querySelector(".kanban-agent-chip")?.hasAttribute("data-hover-tip")).toBe(false);
        expect(seatOfFn).not.toHaveBeenCalled();
    });

    it("keeps the name plain when the agent has no colour, and coloured when it has", () => {
        const { container } = render(() => <AgentChip name="Amy" />);
        expect(container.querySelector(".kanban-agent-chip__name")?.getAttribute("style")).toBeNull();
        expect(container.querySelector(".ui-status-dot")?.getAttribute("aria-label")).toBe("Amy: not here");
        cleanup();
        colorOf.mockReturnValue("#fff");
        const coloured = render(() => <AgentChip name="Amy" colors={{ a: "b" }} uid="u1" />);
        expect(
            (coloured.container.querySelector(".kanban-agent-chip__name") as HTMLElement).style.getPropertyValue(
                "--kanban-agent-text"
            )
        ).toBe("#fff");
        expect(colorOf).toHaveBeenCalledWith(undefined, "u1", { a: "b" }, "identity");
    });
});

describe("KanbanCard: the reason and the note", () => {
    it("says why a task needs you, with an alert icon, and hides the note then", () => {
        const { container } = mount(task(5, "blocked", {}, { question: "which one?", note: "a note" }));
        const reason = container.querySelector(".kanban-card__reason") as HTMLElement;
        expect(reason).toHaveTextContent(/^asks: which one\?$/);
        expect(reason.querySelector("i")).toHaveClass("fa-circle-exclamation");
        expect(container.querySelector(".kanban-card__note")).toBeNull();
    });

    it("shows the note when there is no reason", () => {
        const { container } = mount(task(5, "open", {}, { note: "remember the tests" }));
        expect(container.querySelector(".kanban-card__note")).toHaveTextContent(/^remember the tests$/);
        expect(container.querySelector(".kanban-card__reason")).toBeNull();
    });

    it("shows neither when there is neither", () => {
        const { container } = mount(task(5, "open"));
        expect(container.querySelector(".kanban-card__note")).toBeNull();
        expect(container.querySelector(".kanban-card__reason")).toBeNull();
    });
});

describe("KanbanCard: its pills", () => {
    it("shows none for a plain waiting task", () => {
        expect(pills(mount(task(5, "open")).container)).toEqual([]);
    });

    it("says a held task is on hold", () => {
        expect(pillWith(mount(task(5, "held")).container, "on hold")).toMatchObject({ icon: "pause", tone: "neutral" });
    });

    it("names the spec or research a task comes from, trimmed and lowercased, with how far it is", () => {
        const { container } = mount(
            task(
                5,
                "open",
                {},
                {
                    after: [
                        link(7, "review", " Spec ", "the spec"),
                        link(8, "done", "RESEARCH", "the study"),
                        link(9, "open", "build"),
                    ],
                }
            )
        );
        expect(pillWith(container, "spec #7 · in review")).toMatchObject({
            icon: "file-lines",
            title: " Spec  #7 the spec",
        });
        expect(pillWith(container, "research #8 · accepted")).toMatchObject({
            icon: "file-lines",
            title: "RESEARCH #8 the study",
        });
        expect(pillTexts(container).filter((t) => t?.includes("#9") && t.includes("·"))).toEqual([]);
    });

    it("says each state a link can be in, and a state it has no word for as itself", () => {
        const states: [KanbanTask["state"], string][] = [
            ["open", "waiting"],
            ["held", "on hold"],
            ["claimed", "in progress"],
            ["blocked", "needs you"],
            ["review", "in review"],
            ["done", "accepted"],
            ["failed", "failed"],
            ["cancelled", "cancelled"],
        ];
        const { container } = mount(task(5, "open", {}, { after: states.map(([s], i) => link(20 + i, s, "spec")) }));
        expect(pillTexts(container).filter((t) => t?.startsWith("spec"))).toEqual(
            states.map(([, word], i) => `spec #${20 + i} · ${word}`)
        );
        cleanup();
        const odd = mount(task(6, "open", {}, { after: [link(30, "paused" as KanbanTask["state"], "spec")] }));
        expect(pillTexts(odd.container)).toContain("spec #30 · paused");
    });

    it("says what a waiting task starts after, leaving out specs and what is finished", () => {
        const after = [
            link(3, "claimed", "build", "first"),
            link(4, "open", undefined, "second"),
            link(6, "done", "build", "done one"),
            link(7, "cancelled", "build", "dropped"),
            link(8, "claimed", "spec", "a spec"),
        ];
        const { container } = mount(task(5, "open", {}, { after }));
        expect(pillWith(container, "after #3, #4")).toMatchObject({
            icon: "hourglass-half",
            title: "Starts after #3 first, #4 second: nobody takes it until then",
        });
    });

    it("says what a task starts after when it is one task", () => {
        const { container } = mount(task(5, "open", {}, { after: [link(3, "claimed", "build", "first")] }));
        expect(pillWith(container, "after #3")).toMatchObject({
            title: "Starts after #3 first: nobody takes it until then",
        });
    });

    it("says nothing of what it starts after once it is not waiting, or when only specs are left", () => {
        const after = [link(3, "claimed", "build", "first")];
        const claimed = mount(
            task(5, "claimed", { claimed_by: "Amy", claimed_by_uid: "u", holder: agent("Amy") }, { after })
        );
        expect(pillTexts(claimed.container).some((t) => t?.startsWith("after"))).toBe(false);
        cleanup();
        const specOnly = mount(task(6, "open", {}, { after: [link(8, "claimed", "spec")] }));
        expect(pillTexts(specOnly.container).some((t) => t?.startsWith("after"))).toBe(false);
        cleanup();
        const finished = mount(task(7, "open", {}, { after: [link(9, "done", "build")] }));
        expect(pillTexts(finished.container).some((t) => t?.startsWith("after"))).toBe(false);
    });

    it("lists a spec's builds, and only a spec's", () => {
        const followers = [link(11, "open", "build", "one"), link(12, "claimed", "build", "two")];
        const spec = mount(
            task(
                5,
                "claimed",
                { kind: "spec", claimed_by: "Amy", claimed_by_uid: "u", holder: agent("Amy") },
                { followers }
            )
        );
        expect(pillWith(spec.container, "build #11")).toMatchObject({ icon: "arrow-right", title: "#11 one: waiting" });
        expect(pillWith(spec.container, "build #12")).toMatchObject({ title: "#12 two: in progress" });
        cleanup();
        const build = mount(task(6, "open", { kind: "build" }, { followers }));
        expect(pillTexts(build.container).some((t) => t?.startsWith("build #"))).toBe(false);
        cleanup();
        const research = mount(task(7, "open", { kind: "Research" }, { followers }));
        expect(pillTexts(research.container)).toContain("build #11");
    });

    it("says what a task is while it needs you or is in review", () => {
        const needs = mount(task(5, "blocked", { kind: "spec" }));
        expect(pillWith(needs.container, "spec")).toMatchObject({ title: "What this task is" });
        cleanup();
        expect(pillTexts(mount(task(6, "review")).container)).toContain("build");
        cleanup();
        expect(pillTexts(mount(task(7, "failed", { kind: "Research" })).container)).toContain("research");
        cleanup();
        expect(pillTexts(mount(task(8, "open")).container)).not.toContain("build");
        cleanup();
        expect(pillTexts(mount(task(9, "done")).container)).not.toContain("build");
    });

    it("says which release shipped it", () => {
        const { container } = mount(
            task(5, "done", {}, { pr: { released_in: "v0.60.19", merged_at: 5 } as KanbanCardFields["pr"] })
        );
        expect(pillWith(container, "v0.60.19")).toMatchObject({
            icon: "box",
            tone: "ok",
            title: "Shipped in this release",
        });
    });

    it("says its PR merged, in the merged column only", () => {
        const merged = { merged_at: 5 } as KanbanCardFields["pr"];
        expect(pillWith(mount(task(5, "done", {}, { pr: merged })).container, "merged")).toMatchObject({
            icon: "code-merge",
            title: "Its PR merged",
        });
        cleanup();
        const inReview = mount(task(6, "review", {}, { pr: merged }));
        expect(pillTexts(inReview.container)).not.toContain("merged");
        cleanup();
        expect(pillTexts(mount(task(7, "done")).container)).not.toContain("merged");
    });

    it("says a task is cancelled", () => {
        expect(pillWith(mount(task(5, "cancelled")).container, "cancelled")).toMatchObject({ icon: "ban" });
    });

    it("says who accepted it, and what they said", () => {
        const said = mount(task(5, "done", { accepted_by: { name: "Amy", said: "looks right", at: 1 } }));
        expect(pillWith(said.container, "accepted by Amy")).toMatchObject({
            icon: "user-check",
            title: "Accepted by Amy. looks right",
        });
        cleanup();
        const silent = mount(task(6, "done", { accepted_by: { name: "Bea", said: "", at: 1 } }));
        expect(pillWith(silent.container, "accepted by Bea")).toMatchObject({ title: "Accepted by Bea." });
        cleanup();
        expect(pillTexts(mount(task(7, "done")).container).some((t) => t?.startsWith("accepted by"))).toBe(false);
    });

    it("shows how much of the checklist is done", () => {
        const checklist = [
            { text: "a", status: "completed" },
            { text: "b", status: "pending" },
            { text: "c", status: "completed" },
        ] as KanbanCardFields["checklist"];
        expect(pillWith(mount(task(5, "open", {}, { checklist })).container, "2/3")).toMatchObject({
            icon: "list-check",
        });
        cleanup();
        expect(pills(mount(task(6, "open")).container)).toEqual([]);
    });

    it("links its PR by number, or as PR", () => {
        const pr = mount(task(5, "review", {}, { pr_url: "https://github.com/o/r/pull/6723" }));
        expect(pillWith(pr.container, "#6723")).toMatchObject({
            icon: "code-pull-request",
            tone: "accent",
            title: "https://github.com/o/r/pull/6723",
        });
        cleanup();
        const other = mount(task(6, "review", {}, { pr_url: "https://example.com/x" }));
        expect(pillWith(other.container, "PR")).toMatchObject({ title: "https://example.com/x" });
    });

    it("says how many times it was taken once that is more than once", () => {
        const twice = mount(task(5, "open", { attempts: 2, max_attempts: 3 }));
        expect(pillWith(twice.container, "attempt 2/3")).toMatchObject({
            title: "Taken 2 times; it is parked as failed after 3 attempts",
        });
        cleanup();
        expect(pillTexts(mount(task(6, "open", { attempts: 1 })).container).some((t) => t?.startsWith("attempt"))).toBe(
            false
        );
        cleanup();
        expect(pillTexts(mount(task(7, "open", { attempts: 0 })).container).some((t) => t?.startsWith("attempt"))).toBe(
            false
        );
    });

    it("names its repository, a local folder by its last part", () => {
        const gh = mount(task(5, "open", {}, { repo: "agentmuxai/agentmux" }));
        expect(gh.container.querySelector(".kanban-card__repo")).toHaveTextContent(/^agentmuxai\/agentmux$/);
        cleanup();
        const posix = mount(task(6, "open", {}, { repo: "local:/home/me/work/proj" }));
        expect(posix.container.querySelector(".kanban-card__repo")).toHaveTextContent(/^proj$/);
        cleanup();
        const win = mount(task(7, "open", {}, { repo: "local:C:\\Users\\me\\proj" }));
        expect(win.container.querySelector(".kanban-card__repo")).toHaveTextContent(/^proj$/);
        cleanup();
        expect(mount(task(8, "open")).container.querySelector(".kanban-card__repo")).toBeNull();
    });

    it("keeps its pills in a fixed order", () => {
        const t = task(
            5,
            "review",
            { attempts: 2, accepted_by: { name: "Amy", said: "", at: 1 }, kind: "spec" },
            {
                after: [link(7, "done", "spec"), link(3, "claimed", "build", "first")],
                followers: [link(11, "open", "build", "one")],
                pr: { released_in: "v1", merged_at: 1 } as KanbanCardFields["pr"],
                checklist: [{ text: "a", status: "completed" }] as KanbanCardFields["checklist"],
                pr_url: "https://github.com/o/r/pull/9",
                repo: "o/r",
            }
        );
        const { container } = mount(t);
        expect(pillTexts(container)).toEqual([
            "spec #7 · accepted",
            "build #11",
            "spec",
            "v1",
            "accepted by Amy",
            "1/1",
            "#9",
            "attempt 2/3",
        ]);
        expect(container.querySelector(".kanban-card__meta")!.lastElementChild).toHaveClass("kanban-card__repo");
    });
});

describe("KanbanCard: the foot", () => {
    it("says who asked for it and how long since it moved", () => {
        const { container } = mount(task(5, "open", {}, { requested_by_kind: "operator" }), { now: NOW });
        const foot = container.querySelector(".kanban-card__foot") as HTMLElement;
        expect(foot.children[0]).toHaveTextContent(/^from you$/);
        expect(foot.children[1]).toHaveTextContent(/^5m$/);
        expect(foot.children[1].getAttribute("data-hover-tip")).toBe("Since it last moved");
    });

    it("says an agent asked, by name", () => {
        const { container } = mount(task(5, "open", {}, { requested_by_kind: "agent", requested_by: "Amy" }));
        expect(container.querySelector(".kanban-card__foot span")).toHaveTextContent(/^from Amy$/);
    });

    it("counts the age from now, and never below nothing", () => {
        const fresh = mount(task(5, "open", { updated_at: NOW + 5 * MIN }), { now: NOW });
        expect(fresh.container.querySelectorAll(".kanban-card__foot span")[1]).toHaveTextContent(/^0s$/);
        cleanup();
        const old = mount(task(6, "open", { updated_at: NOW - 3 * 3_600_000 }), { now: NOW });
        expect(old.container.querySelectorAll(".kanban-card__foot span")[1]).toHaveTextContent(/^3h$/);
    });

    it("counts the age from when it last moved, not from when it was made", () => {
        const { container } = mount(task(5, "open", { created_at: NOW - 5 * 3_600_000, updated_at: NOW - 2 * MIN }), {
            now: NOW,
        });
        expect(container.querySelectorAll(".kanban-card__foot span")[1]).toHaveTextContent(/^2m$/);
    });

    it("follows the clock it is given", () => {
        const { container } = mount(task(5, "open", { updated_at: 1000 }), { now: 1000 + 42_000 });
        expect(container.querySelectorAll(".kanban-card__foot span")[1]).toHaveTextContent(/^42s$/);
    });
});

describe("UntrackedCard", () => {
    function untracked(a: KanbanAgent = agent("Amy", { state: "working" })) {
        const onTrack = vi.fn();
        const onOpenAgent = vi.fn();
        const result = render(() => <UntrackedCard agent={a} onTrack={onTrack} onOpenAgent={onOpenAgent} />);
        return { ...result, onTrack, onOpenAgent, a };
    }

    it("is a dashed card with the agent's name, its state and 'untracked'", () => {
        const { container } = untracked();
        const card = container.querySelector(".kanban-card") as HTMLElement;
        expect(card).toHaveClass("kanban-card--untracked");
        expect(card.querySelector(".kanban-agent-chip__name")).toHaveTextContent(/^Amy$/);
        expect(card.querySelector(".ui-status-dot")?.getAttribute("aria-label")).toBe("Amy: working");
        expect(card.querySelector(".kanban-card__untracked-label")).toHaveTextContent(/^untracked$/);
    });

    it("shows no seat for the agent, though it has one", () => {
        seatOfFn.mockReturnValue({ name: "lead" });
        const { container } = untracked();
        expect(container.querySelector(".kanban-agent-chip")?.hasAttribute("data-hover-tip")).toBe(false);
        expect(seatOfFn).not.toHaveBeenCalled();
    });

    it("says what the agent is doing, from its pane's summary", () => {
        blocks["block-Amy"] = { meta: { "term:ambient_summary": "  fixing the build  ", "term:last_prompt": "do it" } };
        expect(untracked().container.querySelector(".kanban-card__text")).toHaveTextContent(/^fixing the build$/);
    });

    it("falls back to its last prompt, then to 'working'", () => {
        blocks["block-Amy"] = { meta: { "term:ambient_summary": "   ", "term:last_prompt": " run the tests " } };
        expect(untracked().container.querySelector(".kanban-card__text")).toHaveTextContent(/^run the tests$/);
        cleanup();
        blocks["block-Amy"] = { meta: { "term:ambient_summary": 5, "term:last_prompt": null } };
        expect(untracked().container.querySelector(".kanban-card__text")).toHaveTextContent(/^working$/);
        cleanup();
        blocks["block-Amy"] = { meta: {} };
        expect(untracked().container.querySelector(".kanban-card__text")).toHaveTextContent(/^working$/);
        cleanup();
        blocks["block-Amy"] = undefined;
        expect(untracked().container.querySelector(".kanban-card__text")).toHaveTextContent(/^working$/);
    });

    it("offers Track and Open, each handing back the agent", () => {
        const { onTrack, onOpenAgent, a } = untracked();
        const track = screen.getByRole("button", { name: "Track" });
        const open = screen.getByRole("button", { name: "Open" });
        expect(track).toHaveClass("ui-density-compact");
        expect(open).toHaveClass("ui-density-compact", "ui-tone-quiet");
        expect(track.querySelector("i")).toHaveClass("fa-thumbtack");
        expect(open.querySelector("i")).toHaveClass("fa-arrow-up-right-from-square");
        fireEvent.click(track);
        expect(onTrack).toHaveBeenCalledWith(a);
        expect(onOpenAgent).not.toHaveBeenCalled();
        fireEvent.click(open);
        expect(onOpenAgent).toHaveBeenCalledWith(a);
        expect(onTrack).toHaveBeenCalledTimes(1);
    });
});
