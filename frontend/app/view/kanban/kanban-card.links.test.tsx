// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A card's words are links (SPEC_LINKS_ON_EVERYTHING_2026_11_20.md §7, build 2): the title, the note and the
// reason name agents and tasks, the holder's chip is the agent's link, and pressing a link never opens the card.

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { opened } = vi.hoisted(() => ({ opened: vi.fn(async () => ({ ok: true as const })) }));

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({ draggable: () => () => {} }));
vi.mock("./kanban-media", () => ({ CardMedia: () => <div /> }));
vi.mock("./kanban-colors", () => ({ agentColor: () => undefined }));
vi.mock("./kanban-seats", () => ({ useSeatIndex: () => () => ({ byUid: new Map(), uidByName: new Map() }), seatOf: () => null }));
vi.mock("@/app/agent-model/seat-format", () => ({ seatTooltipLine: () => "" }));
vi.mock("@/app/store/global", () => ({ MOS: { getMuxObjectAtom: () => () => undefined } }));
vi.mock("@/app/store/agent-names", async () => {
    const { createSignal } = await import("solid-js");
    const [agentRoster, setRoster] = createSignal<{ id: string; slug: string; name: string }[]>([]);
    return { followAgentNames: () => {}, agentRoster, setRoster };
});
vi.mock("@/app/store/misc-utils", () => ({ getHostName: () => "narko" }));
vi.mock("@/app/store/float-modifier", () => ({ takeFloatRequest: () => false }));
vi.mock("@/app/element/link-open", () => ({ onLinkClick: () => {}, onLinkAuxClick: () => {} }));
vi.mock("@/app/links/entities/registry", async (orig) => ({
    ...(await orig<typeof import("@/app/links/entities/registry")>()),
    openEntity: opened,
}));

import * as names from "@/app/store/agent-names";
import { publishTasks } from "@/app/links/task-index-store";
import { AgentChip, KanbanCard } from "./kanban-card";
import type { KanbanCardFields, KanbanTask } from "./kanban-types";

const setRoster = (names as unknown as { setRoster: (e: { id: string; slug: string; name: string }[]) => void }).setRoster;

function task(title: string, card: Partial<KanbanCardFields> = {}, over: Partial<KanbanTask> = {}): KanbanTask {
    return {
        id: "w1",
        title,
        payload: "",
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
        holder: null,
        result: "",
        card: {
            num: 1,
            repo: "agentmuxai/agentmux",
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
            updated_at: 1,
            ...card,
        },
        ...over,
    } as KanbanTask;
}

beforeEach(() => {
    setRoster([{ id: "opaz-0909m", slug: "opaz", name: "Opaz" }]);
    publishTasks([{ board: "main", number: 48247, title: "t", state: "open" }], "main");
    opened.mockClear();
});
afterEach(cleanup);

describe("a card's words are links", () => {
    it("links the agent and the task in its title and its note", () => {
        const onOpen = vi.fn();
        const { container } = render(() => (
            <KanbanCard task={task("Opaz checks #48247", { note: "waiting on #48247" })} now={2} agents={[]} onOpen={onOpen} />
        ));
        const title = [...container.querySelectorAll(".kanban-card__text a")].map((a) => a.textContent);
        expect(title).toEqual(["Opaz", "#48247"]);
        expect([...container.querySelectorAll(".kanban-card__note a")].map((a) => a.textContent)).toEqual(["#48247"]);
    });

    it("opens what the link names, and not the card", () => {
        const onOpen = vi.fn();
        const { container } = render(() => <KanbanCard task={task("see #48247")} now={2} agents={[]} onOpen={onOpen} />);
        const a = container.querySelector(".kanban-card__text a") as HTMLAnchorElement;
        fireEvent.click(a);
        fireEvent.keyDown(a, { key: "Enter" });
        expect(opened).toHaveBeenCalledTimes(1);
        expect(onOpen).not.toHaveBeenCalled();
    });

    it("still opens the card on a press on its own text", () => {
        const onOpen = vi.fn();
        const { container } = render(() => <KanbanCard task={task("plain words")} now={2} agents={[]} onOpen={onOpen} />);
        fireEvent.click(container.querySelector(".kanban-card") as HTMLElement);
        expect(onOpen).toHaveBeenCalledTimes(1);
    });

    it("keeps the title as it was when the roster and the board name nothing in it", () => {
        setRoster([]);
        const { container } = render(() => <KanbanCard task={task("Opaz #999999")} now={2} agents={[]} onOpen={vi.fn()} />);
        expect(container.querySelectorAll(".kanban-card__text a")).toHaveLength(0);
        expect(container.querySelector(".kanban-card__text")?.textContent?.trim()).toBe("Opaz #999999");
    });
});

describe("the agent chip is the agent's link", () => {
    it("links the name when the agent's uid is known, and not when it is not", () => {
        const { container } = render(() => (
            <>
                <AgentChip name="tess" display="Tess" uid="u-tess" />
                <AgentChip name="gone" display="Gone" />
            </>
        ));
        const links = [...container.querySelectorAll(".kanban-agent-chip a")];
        expect(links.map((a) => a.textContent)).toEqual(["Tess"]);
        fireEvent.click(links[0]);
        expect(opened).toHaveBeenCalledWith({ kind: "agent", id: "u-tess", name: "Tess" }, { floating: false });
    });
});
