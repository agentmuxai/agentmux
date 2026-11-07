// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the Kanban section does around its board, pinned as the section's markup and ordered logs: the
 * board loads while the section is shown, its toolbar filters feed the board, lanes and folding, the
 * collapsed columns (the pane's, with a pending choice, or its own), a task opened from elsewhere, the
 * agent strip, the narrow layout and the Needs-you count. The real section runs over a real
 * `KanbanModel`; the board, the drawer and the new-task form are stand-ins that show the props they got.
 * A refactor of the section must leave the committed snapshots as they are; regenerate with `vitest -u`
 * only for a change meant to behave differently.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal, onCleanup, onMount } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    log: [] as string[],
    board: null as any,
    listFails: false,
    episodes: [] as any[],
    defs: [] as any[],
    settings: {} as Record<string, unknown>,
    drops: [] as ((id: string) => void)[],
    resize: null as null | ((width: number) => void),
    boards: 0,
    needs: null as number | null,
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        KanbanListCommand: async () => {
            h.log.push("rpc KanbanList");
            if (h.listFails) throw new Error("srv is down");
            return h.board;
        },
        KanbanActCommand: async (_c: unknown, req: unknown) => {
            h.log.push(`rpc KanbanAct ${JSON.stringify(req)}`);
            return {};
        },
        IdentityAccountAvailabilityCommand: async () => ({ episodes: h.episodes }),
        ListAgentDefinitionsCommand: async () => h.defs,
        SetConfigCommand: async (_c: unknown, v: unknown) => void h.log.push(`rpc SetConfig ${JSON.stringify(v)}`),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (type: string) => {
        h.log.push(`subscribe ${type}`);
        return () => h.log.push(`unsubscribe ${type}`);
    },
}));
vi.mock("@/app/store/config-signals", async (orig) => ({
    ...(await orig<typeof import("@/app/store/config-signals")>()),
    settingsAtom: () => h.settings,
}));
vi.mock("@/app/util/reveal-block", () => ({
    revealBlock: (id: string, opts: unknown) => void h.log.push(`revealBlock ${id} ${JSON.stringify(opts)}`),
}));
vi.mock("./kanban-card", () => ({
    AgentChip: (p: { name: string }) => <span class="stub-chip">{p.name}</span>,
    agentStateWords: (s: string) => s,
    KanbanCard: () => null,
    UntrackedCard: () => null,
}));
vi.mock("./kanban-board", () => ({
    useCardDrop: (_el: unknown, onDrop: (id: string) => void) => {
        h.drops.push(onDrop);
        return () => false;
    },
    KanbanBoard: (p: any) => {
        const id = ++h.boards;
        onMount(() => h.log.push(`board#${id} mounted`));
        onCleanup(() => h.log.push(`board#${id} unmounted`));
        const shown = () =>
            JSON.stringify({
                tasks: p.tasks.map((t: any) => t.id),
                untracked: p.untracked.map((a: any) => a.name),
                agents: p.agents.length,
                now: typeof p.now,
                holdingCamper: p.holding("Camper")?.id ?? null,
                holdingKorp: p.holding("Korp")?.id ?? null,
                reviewLimit: p.reviewLimit,
                reviewTotal: p.reviewTotal,
                colors: p.colors ?? null,
                narrow: p.narrow,
                collapsed: p.collapsed,
            });
        return (
            <div class="stub-board" data-id={id}>
                <pre>{shown()}</pre>
                <button class="stub-board-open" onClick={() => p.onOpen(p.tasks[0])} />
                <button class="stub-board-agent" onClick={() => p.onOpenAgent("blk-from-card")} />
                <button class="stub-board-toggle" onClick={() => p.onToggleCollapsed("review")} />
            </div>
        );
    },
}));
vi.mock("./kanban-drawer", () => ({
    KanbanDrawer: (p: any) => {
        const id = p.task.id;
        onMount(() => h.log.push(`drawer mounted for ${id}`));
        onCleanup(() => h.log.push(`drawer unmounted for ${id}`));
        return (
            <div class="stub-drawer">
                <pre>{JSON.stringify({ task: p.task.id, agents: p.agents.length, now: typeof p.now })}</pre>
                <button class="stub-drawer-close" onClick={() => p.onClose()} />
                <button class="stub-drawer-task" onClick={() => p.onOpenTask("w2")} />
                <button class="stub-drawer-agent" onClick={() => p.onOpenAgent("blk-from-drawer")} />
            </div>
        );
    },
}));
vi.mock("./kanban-new-task", () => ({
    KanbanNewTask: (p: { agents: unknown[]; onDone: () => void }) => {
        onMount(() => h.log.push(`new task form mounted with ${p.agents.length} agents`));
        onCleanup(() => h.log.push("new task form unmounted"));
        return <button class="stub-new-done" onClick={() => p.onDone()} />;
    },
}));

import { KANBAN_NARROW_PX } from "./kanban-columns";
import { KanbanSection } from "./kanban-section";

function task(num: number, state: string, over: Record<string, unknown> = {}, card: Record<string, unknown> = {}) {
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
        created_at: num,
        updated_at: num,
        result: "",
        holder: null,
        card: {
            num,
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
            updated_at: num,
            ...card,
        },
        ...over,
    };
}
const agent = (name: string, state: string) => ({ name, uid: `u-${name.toLowerCase()}`, block_id: `blk-${name.toLowerCase()}`, state });

beforeEach(() => {
    class FakeResizeObserver {
        constructor(cb: (e: { contentRect: { width: number } }[]) => void) {
            h.resize = (width) => cb([{ contentRect: { width } }]);
        }
        observe() {}
        disconnect() {
            h.log.push("resize observer disconnected");
        }
    }
    (globalThis as any).ResizeObserver = FakeResizeObserver;
    h.log.length = 0;
    h.listFails = false;
    h.drops.length = 0;
    h.resize = null;
    h.boards = 0;
    h.needs = null;
    h.settings = {};
    h.episodes = [];
    h.board = {
        tasks: [
            task(1, "open"),
            task(2, "review"),
            task(3, "claimed", { claimed_by: "Camper", claimed_by_uid: "u-camper" }),
            task(4, "blocked", { claimed_by: "Naki-2" }, { repo: "a5af/other" }),
            task(5, "open", { target_agent: "Korp", target_agent_uid: "u-korp" }, { repo: "" }),
            task(6, "review", {}, { repo: "a5af/other" }),
        ],
        agents: [agent("Camper", "working"), agent("Korp", "waiting"), agent("Idle", "idle")],
        agent_colors: { "u-gone": "#112233" },
        stamp: 1,
        now: 1,
    };
    const seat = (role: string, lead: string | null) => ({ role, lead, qualifier: null, area: null });
    h.defs = [
        { id: "u-agentx", name: "AgentX", is_seeded: false, seat: seat("leader", null) },
        { id: "u-camper", name: "Camper", is_seeded: false, seat: seat("specialist", "u-agentx") },
        { id: "u-korp", name: "Korp", is_seeded: false },
    ];
});
afterEach(cleanup);

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const settle = async () => {
    for (let i = 0; i < 4; i++) await sleep(15);
};
type Props = Parameters<typeof KanbanSection>[0];
async function mount(props: Partial<Props> = {}) {
    const view = render(() => <KanbanSection visible onNeedsCount={(n) => (h.needs = n)} {...props} />);
    await settle();
    return view;
}
/** The section's markup, with the generated ids blanked and one tag to a line. */
const html = (c: HTMLElement) =>
    c.innerHTML
        .replace(/\s(id|for|aria-describedby|aria-labelledby)="[^"]*"/g, "")
        .replace(/></g, ">\n<");
const mark = (label: string, c: HTMLElement) => h.log.push(`-- ${label}\n${html(c)}`);
const out = () => h.log.join("\n") + "\n";
const snap = (name: string) => `./__snapshots__/kanban-section.behavior.${name}.txt`;
const q = <T extends Element>(c: HTMLElement, sel: string) => c.querySelector(sel) as T;
const boards = (c: HTMLElement) => [...c.querySelectorAll(".stub-board pre")].map((p) => p.textContent);

describe("loading and following the board", () => {
    it("shows a note while loading, then the board, the agent strip and the Needs-you count", async () => {
        const view = render(() => <KanbanSection visible onNeedsCount={(n) => (h.needs = n)} />);
        mark("before the board arrives", view.container);
        await settle();
        mark("loaded", view.container);
        h.log.push(`needs count: ${h.needs}`);
        await expect(out()).toMatchFileSnapshot(snap("loading-and-loaded"));
    });

    it("counts an open account episode once, and not the tasks it covers", async () => {
        for (const num of [7, 8])
            h.board.tasks.push(task(num, "claimed", { claimed_by: "Korp", claimed_by_uid: "u-korp", holder: { ...agent("Korp", "limited") } }));
        const without = await mount();
        h.log.push(`needs count, no episode: ${h.needs}`);
        without.unmount();
        h.episodes = [{ agent_ids: ["u-korp"], block_ids: [] }];
        await mount();
        h.log.push(`needs count, an episode covering the limited holder: ${h.needs}`);
        await expect(out()).toMatchFileSnapshot(snap("needs-count-episode"));
    });

    it("says why the board could not load, and the error can be dismissed", async () => {
        h.listFails = true;
        const view = await mount();
        mark("failed", view.container);
        fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
        mark("dismissed", view.container);
        await expect(out()).toMatchFileSnapshot(snap("load-error"));
    });

    it("fetches and subscribes only while shown, and Refresh fetches again", async () => {
        const [visible, setVisible] = createSignal(false);
        const view = render(() => <KanbanSection visible={visible()} />);
        await settle();
        h.log.push("-- hidden: nothing fetched");
        setVisible(true);
        await settle();
        h.log.push("-- shown");
        fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
        await settle();
        h.log.push("-- refreshed");
        setVisible(false);
        await settle();
        h.log.push("-- hidden again");
        view.unmount();
        h.log.push("-- unmounted");
        await expect(out()).toMatchFileSnapshot(snap("visibility-and-refresh"));
    });
});

describe("the toolbar", () => {
    it("filters the tasks the board gets: by text, by repository, by who the owner reports to, and untracked work", async () => {
        const view = await mount();
        const probe = (label: string) => h.log.push(`${label}: ${boards(view.container).join(" ")}`);
        probe("all");
        const find = screen.getByRole("textbox") as HTMLInputElement;
        fireEvent.input(find, { target: { value: "#4" } });
        probe("find #4");
        fireEvent.input(find, { target: { value: "" } });
        fireEvent.change(screen.getByLabelText("Repository"), { target: { value: "a5af/other" } });
        probe("repository a5af/other");
        fireEvent.change(screen.getByLabelText("Repository"), { target: { value: "" } });
        fireEvent.change(screen.getByLabelText("Reports to"), { target: { value: "lead:u-agentx" } });
        probe("reports to AgentX");
        fireEvent.change(screen.getByLabelText("Reports to"), { target: { value: "operator" } });
        probe("reports to the operator");
        fireEvent.change(screen.getByLabelText("Reports to"), { target: { value: "" } });
        fireEvent.click(screen.getByRole("switch", { name: "Show untracked work" }));
        probe("untracked off");
        mark("toolbar", view.container);
        await expect(out()).toMatchFileSnapshot(snap("toolbar-filters"));
    });

    it("turns reminders on and off through the settings", async () => {
        h.settings = { "kanban:reminders": true };
        await mount();
        fireEvent.click(screen.getByRole("switch", { name: "Remind idle agents of their backlog" }));
        await settle();
        await expect(out()).toMatchFileSnapshot(snap("toolbar-reminders"));
    });

    it("opens the new-task form with the agents, and closes it when it is done", async () => {
        const view = await mount();
        fireEvent.click(screen.getByRole("button", { name: "New task" }));
        h.log.push(`form shown: ${!!q(view.container, ".stub-new-done")}`);
        fireEvent.click(q(view.container, ".stub-new-done"));
        h.log.push(`form shown: ${!!q(view.container, ".stub-new-done")}`);
        await expect(out()).toMatchFileSnapshot(snap("new-task"));
    });
});

describe("lanes", () => {
    it("shows a row per repository, with the untracked work first, and a lane folds to its head", async () => {
        const view = await mount();
        fireEvent.click(screen.getByRole("switch", { name: "Lanes by repository" }));
        mark("lanes on", view.container);
        fireEvent.click(q(view.container, "button.kanban-lane__head"));
        mark("first lane folded", view.container);
        fireEvent.click(q(view.container, "button.kanban-lane__head"));
        mark("first lane open again", view.container);
        fireEvent.click(screen.getByRole("switch", { name: "Lanes by repository" }));
        h.log.push(`lanes off, boards: ${boards(view.container).length}`);
        await expect(out()).toMatchFileSnapshot(snap("lanes"));
    });

    it("keeps a lane's board mounted when the board refreshes", async () => {
        const view = await mount();
        fireEvent.click(screen.getByRole("switch", { name: "Lanes by repository" }));
        h.log.push("-- refresh");
        h.board = { ...h.board, tasks: [...h.board.tasks, task(8, "open")], stamp: 2 };
        fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
        await settle();
        h.log.push(boards(view.container).join("\n"));
        await expect(out()).toMatchFileSnapshot(snap("lanes-refresh"));
    });

    it("follows the pane's lanes setting when given, and asks the pane to change it", async () => {
        const [lanes, setLanes] = createSignal(false);
        const onLanes = vi.fn((on: boolean) => h.log.push(`pane asked for lanes ${on}`));
        const view = render(() => <KanbanSection visible lanes={lanes()} onLanes={onLanes} />);
        await settle();
        const sw = () => screen.getByRole("switch", { name: "Lanes by repository" });
        fireEvent.click(sw());
        h.log.push(`after the click, still checked: ${sw().getAttribute("aria-checked")}`);
        setLanes(true);
        await settle();
        h.log.push(`pane set lanes, checked: ${sw().getAttribute("aria-checked")}, lanes: ${view.container.querySelectorAll(".kanban-lane").length}`);
        await expect(out()).toMatchFileSnapshot(snap("lanes-pane"));
    });
});

describe("collapsed columns", () => {
    it("are the section's own when it mounts alone", async () => {
        const view = await mount();
        const board = () => boards(view.container)[0];
        fireEvent.click(q(view.container, ".stub-board-toggle"));
        h.log.push(`own, collapsed once: ${board()}`);
        fireEvent.click(q(view.container, ".stub-board-toggle"));
        h.log.push(`own, collapsed twice: ${board()}`);
        await expect(out()).toMatchFileSnapshot(snap("collapsed-own"));
    });

    it("are the pane's, and a choice shows at once and holds until the pane says the same", async () => {
        const [collapsed, setCollapsed] = createSignal<string[]>([]);
        const onCollapsed = vi.fn((ids: string[]) => h.log.push(`pane told ${JSON.stringify(ids)}`));
        const view = render(() => <KanbanSection visible collapsed={collapsed()} onCollapsed={onCollapsed} />);
        await settle();
        const board = () => boards(view.container)[0]!.match(/"collapsed":\[[^\]]*\]/)![0];
        fireEvent.click(q(view.container, ".stub-board-toggle"));
        h.log.push(`pending: ${board()}`);
        setCollapsed(["review"]);
        await settle();
        h.log.push(`pane agrees: ${board()}`);
        setCollapsed([]);
        await settle();
        h.log.push(`pane changes it on its own: ${board()}`);
        await expect(out()).toMatchFileSnapshot(snap("collapsed-pane"));
    });

    it("drop a pending choice after three seconds if the pane never says the same", async () => {
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
        try {
            const view = render(() => <KanbanSection visible collapsed={[]} onCollapsed={() => {}} />);
            await vi.advanceTimersByTimeAsync(0);
            const board = () => boards(view.container)[0]!.match(/"collapsed":\[[^\]]*\]/)![0];
            fireEvent.click(q(view.container, ".stub-board-toggle"));
            h.log.push(`pending: ${board()}`);
            await vi.advanceTimersByTimeAsync(2900);
            h.log.push(`after 2.9 s: ${board()}`);
            await vi.advanceTimersByTimeAsync(200);
            h.log.push(`after 3.1 s: ${board()}`);
        } finally {
            vi.useRealTimers();
        }
        await expect(out()).toMatchFileSnapshot(snap("collapsed-pending-times-out"));
    });
});

describe("opening a task and an agent", () => {
    it("opens the task a request names, by id or by number, once the board has it, and clears the request", async () => {
        const [request, setRequest] = createSignal<string | null>(null);
        const onOpenHandled = vi.fn(() => {
            h.log.push("request handled");
            setRequest(null);
        });
        const view = render(() => <KanbanSection visible openRequest={request()} onOpenHandled={onOpenHandled} />);
        await settle();
        h.log.push("-- a request for a task the board lacks");
        setRequest("w99");
        await settle();
        h.log.push(`drawer: ${!!q(view.container, ".stub-drawer")}`);
        h.log.push("-- a request by number");
        setRequest("3");
        await settle();
        h.log.push(q(view.container, ".stub-drawer pre").textContent!);
        fireEvent.click(q(view.container, ".stub-drawer-task"));
        h.log.push(q(view.container, ".stub-drawer pre").textContent!);
        fireEvent.click(q(view.container, ".stub-drawer-close"));
        h.log.push(`drawer: ${!!q(view.container, ".stub-drawer")}`);
        await expect(out()).toMatchFileSnapshot(snap("open-request"));
    });

    it("opens a card's drawer, and an agent through the pane's handler or by revealing it", async () => {
        const onOpenAgent = vi.fn((id: string) => h.log.push(`pane places ${id}`));
        const first = await mount({ onOpenAgent });
        fireEvent.click(q(first.container, ".stub-board-open"));
        h.log.push(q(first.container, ".stub-drawer pre").textContent!);
        fireEvent.click(q(first.container, ".stub-drawer-agent"));
        fireEvent.click(q(first.container, ".stub-board-agent"));
        fireEvent.click(screen.getByTitle(/^Camper: working\./));
        first.unmount();
        h.log.push("-- mounted alone");
        const alone = await mount();
        fireEvent.click(q(alone.container, ".stub-board-agent"));
        fireEvent.click(screen.getByTitle(/^Korp: waiting\./));
        await expect(out()).toMatchFileSnapshot(snap("open-agent"));
    });

    it("starts a waiting task on the agent whose chip it is dropped on", async () => {
        await mount();
        h.drops[1]!("w1");
        await settle();
        await expect(out()).toMatchFileSnapshot(snap("drop-on-agent"));
    });
});

describe("the narrow layout", () => {
    it("follows the section's own width, at the limit and a pixel over", async () => {
        const view = await mount();
        const narrow = () => boards(view.container)[0]!.match(/"narrow":(true|false)/)![0];
        h.log.push(`start: ${narrow()}`);
        h.resize!(KANBAN_NARROW_PX);
        h.log.push(`${KANBAN_NARROW_PX} px: ${narrow()}`);
        h.resize!(KANBAN_NARROW_PX + 1);
        h.log.push(`${KANBAN_NARROW_PX + 1} px: ${narrow()}`);
        view.unmount();
        await expect(out()).toMatchFileSnapshot(snap("narrow"));
    });
});
