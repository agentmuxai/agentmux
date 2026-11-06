// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The task drawer end to end: what each part shows for each kind of task, and what each button asks srv
 * to do. Written to pin the behaviour before KanbanDrawer is split into smaller components.
 */
import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    createBlock: vi.fn(),
    openLink: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListAgentDefinitionsCommand: () => Promise.resolve([]) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/store/misc-utils", () => ({ openLink: h.openLink }));
vi.mock("@/app/store/block-layout-actions", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/block-layout-actions")>()),
    createBlock: h.createBlock,
}));
// Newest-first is the live order; reversed here so the drawer is seen to use it.
vi.mock("./kanban-agent-order", () => ({ agentsByRecentUse: (a: unknown[]) => [...a].reverse() }));

import { KanbanDrawer } from "./kanban-drawer";
import type { KanbanModel } from "./kanban-model";
import type {
    KanbanActRequest,
    KanbanActResult,
    KanbanAfter,
    KanbanAgent,
    KanbanEvent,
    KanbanMediaItem,
    KanbanTask,
    KanbanTaskDetail,
} from "./kanban-types";

const naki: KanbanAgent = { name: "Naki-2", uid: "uid-n", block_id: "b-n", state: "working" };
const loap: KanbanAgent = { name: "Loap-3", uid: "uid-l", block_id: "b-l", state: "idle" };
const clare: KanbanAgent = { name: "Clare", uid: "uid-c", block_id: "b-c", state: "waiting" };
const gone: KanbanAgent = { name: "Ghost", uid: "uid-g", block_id: "b-g", state: "gone" };
const stopped: KanbanAgent = { name: "Sleeper", uid: "uid-s", block_id: "b-s", state: "stopped" };

const NOW = 600_000;

function task(over: Partial<KanbanTask> = {}, card: Partial<KanbanTask["card"]> = {}): KanbanTask {
    return {
        id: "w1",
        title: "Fix the thing",
        payload: "do it",
        kind: "",
        target_agent: "",
        target_agent_uid: "",
        priority: 0,
        state: "open",
        claimed_by: "",
        claimed_by_uid: "",
        attempts: 1,
        max_attempts: 3,
        created_by: "operator",
        created_at: 1,
        updated_at: 1,
        result: "",
        holder: null,
        holder_unavailable: null,
        card: {
            num: 12,
            repo: "",
            branch: "",
            pr_url: "",
            review_required: false,
            checklist: [],
            question: "",
            note: "",
            requested_by: "",
            requested_by_kind: "operator",
            request_text: "",
            filed_by: "",
            updated_at: 1,
            ...card,
        },
        ...over,
    };
}

const claimed = (over: Partial<KanbanTask> = {}, card: Partial<KanbanTask["card"]> = {}) =>
    task({ state: "claimed", claimed_by: "Naki-2", claimed_by_uid: "uid-n", holder: naki, ...over }, card);

function after(num: number, over: Partial<KanbanAfter> = {}): KanbanAfter {
    return { id: `a${num}`, num, title: `Task ${num}`, state: "open", ...over };
}

function event(id: number, over: Partial<KanbanEvent> = {}): KanbanEvent {
    return {
        id,
        work_id: "w1",
        at: NOW - 120_000,
        actor: "Naki-2",
        actor_kind: "agent",
        kind: "claimed",
        from_state: "open",
        to_state: "claimed",
        body: "",
        ...over,
    };
}

function mediaItem(id: number, over: Partial<KanbanMediaItem> = {}): KanbanMediaItem {
    return {
        id,
        work_id: "w1",
        sha256: `sha${id}`,
        name: `shot${id}.png`,
        kind: "image",
        mime: "image/png",
        size: 10,
        caption: "",
        attached_by: "Naki-2",
        attempt: 1,
        at: NOW - 60_000,
        path: "",
        thumb: "",
        ...over,
    };
}

function deferred<T>() {
    let resolve!: (v: T) => void;
    const promise = new Promise<T>((r) => (resolve = r));
    return { promise, resolve };
}

interface Opts {
    answers?: (KanbanActResult | null)[];
    agents?: KanbanAgent[];
    detail?: Partial<KanbanTaskDetail> | Promise<KanbanTaskDetail>;
}

function mount(t: KanbanTask, opts: Opts = {}) {
    const act = vi.fn<(req: KanbanActRequest) => Promise<KanbanActResult | null>>();
    for (const a of opts.answers ?? []) act.mockResolvedValueOnce(a);
    act.mockResolvedValue({ ok: true });
    const detail = vi.fn<(id: string) => Promise<KanbanTaskDetail>>(() =>
        opts.detail instanceof Promise ? opts.detail : Promise.resolve({ task: t, events: [], ...opts.detail })
    );
    const model = { act, detail, board: () => null } as unknown as KanbanModel;
    const onClose = vi.fn();
    const onOpenAgent = vi.fn();
    const onOpenTask = vi.fn();
    const [current, setTask] = createSignal(t);
    render(() => (
        <KanbanDrawer
            model={model}
            task={current()}
            agents={opts.agents ?? [naki, loap]}
            now={NOW}
            onClose={onClose}
            onOpenAgent={onOpenAgent}
            onOpenTask={onOpenTask}
        />
    ));
    return { act, detail, onClose, onOpenAgent, onOpenTask, setTask };
}

const facts = () => document.querySelector(".kanban-drawer__facts") as HTMLElement;
const actions = () => document.querySelector(".kanban-drawer__actions") as HTMLElement;
const heading = (name: string) => screen.queryByRole("heading", { name });
const optionLabels = (label: string) =>
    [...(screen.getByLabelText(label) as HTMLSelectElement).options].map((o) => o.textContent);
const button = (name: string | RegExp) => screen.getByRole("button", { name }) as HTMLButtonElement;
const lastCall = (act: { mock: { calls: unknown[][] } }) => act.mock.calls[act.mock.calls.length - 1][0];

beforeEach(() => vi.clearAllMocks());
afterEach(cleanup);

describe("KanbanDrawer: the header", () => {
    it("names the task and labels the drawer with its number", () => {
        mount(task());
        expect(screen.getByLabelText("Task 12").tagName).toBe("ASIDE");
        expect(document.querySelector(".kanban-card__num")?.textContent).toBe("#12");
        expect(screen.getByRole("heading", { name: "Fix the thing" }).tagName).toBe("H3");
    });

    it("closes from the Close button", () => {
        const { onClose } = mount(task());
        fireEvent.click(button("Close"));
        expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("follows the task it is given when the board changes it", () => {
        const { setTask } = mount(task());
        setTask(task({ title: "Renamed" }, { num: 99 }));
        expect(screen.getByLabelText("Task 99")).toBeTruthy();
        expect(screen.getByRole("heading", { name: "Renamed" })).toBeTruthy();
    });
});

describe("KanbanDrawer: the facts line", () => {
    it("shows the agent holding the task with its state in words", () => {
        mount(claimed({ holder: { ...naki, state: "limited" } }));
        expect(within(facts()).getByRole("img", { name: "Naki-2: out of quota" })).toBeTruthy();
        expect(facts().textContent).toContain("Naki-2");
        expect(facts().textContent).toContain(" out of quota");
    });

    it("shows the agent a task waits for when nobody holds it, from the running agents", () => {
        mount(task({ target_agent: "Loap-3" }));
        expect(within(facts()).getByRole("img", { name: "Loap-3: idle" })).toBeTruthy();
        expect(facts().textContent).toContain(" idle");
    });

    it("prefers the holder over a running agent of the same name", () => {
        mount(claimed({ holder: { ...naki, state: "waiting" } }), { agents: [{ ...naki, state: "idle" }] });
        expect(within(facts()).getByRole("img", { name: "Naki-2: waiting on a question" })).toBeTruthy();
    });

    it("says an agent it cannot find is not here", () => {
        mount(task({ target_agent: "Nobody" }));
        expect(within(facts()).getByRole("img", { name: "Nobody: not here" })).toBeTruthy();
        expect(facts().textContent).toContain(" not here");
    });

    it("names the claimant before the target", () => {
        mount(claimed({ target_agent: "Loap-3" }));
        expect(within(facts()).queryByRole("img", { name: /^Loap-3/ })).toBeNull();
        expect(within(facts()).getByRole("img", { name: /^Naki-2/ })).toBeTruthy();
    });

    it("shows no agent for a task with neither", () => {
        mount(task());
        expect(within(facts()).queryByRole("img")).toBeNull();
    });

    it("says who asked", () => {
        mount(task({}, { requested_by_kind: "agent", requested_by: "AgentY" }));
        expect(facts().textContent).toContain("asked by AgentY");
    });

    it("shows the repo alone, or with its branch", () => {
        mount(task({}, { repo: "agentmuxai/agentmux" }));
        expect(facts().textContent).toContain("agentmuxai/agentmux");
        expect(facts().textContent).not.toContain("·");
        cleanup();
        mount(task({}, { repo: "agentmuxai/agentmux", branch: "maop/x" }));
        expect(facts().textContent).toContain("agentmuxai/agentmux · maop/x");
    });

    it("shows no branch without a repo", () => {
        mount(task({}, { branch: "maop/x" }));
        expect(facts().textContent).not.toContain("maop/x");
    });

    it("opens the PR through the link opener", () => {
        mount(task({}, { pr_url: "https://github.com/agentmuxai/agentmux/pull/7" }));
        fireEvent.click(button("Open the PR"));
        expect(h.openLink).toHaveBeenCalledWith("https://github.com/agentmuxai/agentmux/pull/7");
    });

    it("offers no PR button without a PR", () => {
        mount(task());
        expect(screen.queryByRole("button", { name: "Open the PR" })).toBeNull();
    });

    it("says when the PR merged, and the release that has it", () => {
        mount(task({}, { pr: { merged_at: NOW - 125_000, released_in: "v0.60.1" } }));
        expect(facts().textContent).toContain("merged 2m ago");
        expect(facts().textContent).toContain(" · released in v0.60.1");
    });

    it("says only that it merged until a release has it", () => {
        mount(task({}, { pr: { merged_at: NOW - 5_000 } }));
        expect(facts().textContent).toContain("merged 5s ago");
        expect(facts().textContent).not.toContain("released in");
    });

    it("never shows a merge from the future", () => {
        mount(task({}, { pr: { merged_at: NOW + 50_000 } }));
        expect(facts().textContent).toContain("merged 0s ago");
    });

    it("says nothing of a merge that has not happened", () => {
        mount(task({}, { pr: { merged_at: null, released_in: "v0.60.1" } }));
        expect(facts().textContent).not.toContain("merged");
        expect(facts().textContent).not.toContain("released");
    });
});

describe("KanbanDrawer: what the task says", () => {
    it("shows why the task needs you", () => {
        mount(task({ state: "blocked" }, { question: "which repo?" }));
        expect(document.querySelector("p.kanban-drawer__reason")?.textContent).toBe("asks: which repo?");
    });

    it("shows no reason for a task that needs nothing", () => {
        mount(task());
        expect(document.querySelector("p.kanban-drawer__reason")).toBeNull();
    });

    it("quotes what you asked", () => {
        mount(task({}, { request_text: "please fix it" }));
        expect(heading("What you asked")).toBeTruthy();
        expect(document.querySelector("blockquote.kanban-drawer__quote")?.textContent).toBe("please fix it");
    });

    it("has no quote when you wrote nothing", () => {
        mount(task());
        expect(heading("What you asked")).toBeNull();
    });

    it("shows the task's own words", () => {
        mount(task({ payload: "line one\nline two" }));
        expect(heading("The task")).toBeTruthy();
        expect(document.querySelector(".kanban-drawer__payload")?.textContent).toBe("line one\nline two");
    });

    it("lists the checklist with each item's status", () => {
        mount(
            task(
                {},
                {
                    checklist: [
                        { text: "write it", status: "completed" },
                        { text: "test it", status: "in_progress" },
                        { text: "ship it", status: "pending" },
                    ],
                }
            )
        );
        expect(heading("Checklist")).toBeTruthy();
        const items = [...document.querySelectorAll(".kanban-checklist li")] as HTMLElement[];
        expect(items.map((li) => li.textContent?.trim())).toEqual(["write it", "test it", "ship it"]);
        expect(items.map((li) => li.className)).toEqual(["kanban-checklist__done", "kanban-checklist__now", ""]);
        expect(items.map((li) => li.querySelector("i")?.className)).toEqual([
            "fa-solid fa-square-check",
            "fa-solid fa-spinner",
            "fa-solid fa-square",
        ]);
        expect(items[0].querySelector("i")?.getAttribute("aria-hidden")).toBe("true");
    });

    it("has no checklist section when the list is empty", () => {
        mount(task());
        expect(heading("Checklist")).toBeNull();
    });
});

describe("KanbanDrawer: starts after", () => {
    it("lists what it waits for, and opens one on click", () => {
        const { onOpenTask } = mount(task({}, { after: [after(5, { title: "Spec it" })] }));
        expect(heading("Starts after")).toBeTruthy();
        fireEvent.click(button("#5 Spec it"));
        expect(onOpenTask).toHaveBeenCalledWith("a5");
    });

    it("edits the list through the model for the task shown", async () => {
        const { act } = mount(task({}, { after: [after(5), after(6)] }));
        fireEvent.click(button("Don't wait for #5"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ action: "edit", id: "w1", after: ["a6"] });
    });

    it("is not shown for a finished task that waited for nothing", () => {
        mount(task({ state: "done" }));
        expect(heading("Starts after")).toBeNull();
    });
});

describe("KanbanDrawer: media", () => {
    it("shows the files from the task's history fetch", async () => {
        mount(task(), { detail: { media: [mediaItem(1, { caption: "the fix" })] } });
        expect(await screen.findByText("shot1.png")).toBeTruthy();
        expect(heading("Media")).toBeTruthy();
        expect(screen.getByText("the fix")).toBeTruthy();
        expect(screen.getByText(/Naki-2, attempt 1, 1m ago/)).toBeTruthy();
    });

    it("has no Media section for a task that has not started and carries nothing", async () => {
        const { detail } = mount(task());
        await waitFor(() => expect(detail).toHaveBeenCalled());
        expect(heading("Media")).toBeNull();
    });

    it("shows the Media section for a started task", () => {
        mount(claimed());
        expect(heading("Media")).toBeTruthy();
    });

    it("flags a UI change through the drawer's move, and reads the flag from the card", async () => {
        const { act } = mount(claimed({}, { media: { count: 0, first: [], ui_change: true, no_media_reason: "" } }));
        const box = screen.getByRole("checkbox", { name: /Changes the UI/ }) as HTMLInputElement;
        expect(box.checked).toBe(true);
        fireEvent.click(box);
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "ui_change", ui_change: false });
    });

    it("removes a file through the drawer's move", async () => {
        const { act } = mount(task(), { detail: { media: [mediaItem(7)] } });
        await screen.findByText("shot7.png");
        fireEvent.click(button("Remove"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "media_remove", media_id: 7 });
    });

    it("keeps the media controls off while a move is in flight", async () => {
        const d = deferred<KanbanActResult | null>();
        const { act } = mount(task(), { detail: { media: [mediaItem(7)] } });
        act.mockReturnValueOnce(d.promise);
        await screen.findByText("shot7.png");
        fireEvent.click(button("Remove"));
        await waitFor(() => expect(button("Remove").disabled).toBe(true));
        expect((screen.getByRole("checkbox", { name: /Changes the UI/ }) as HTMLInputElement).disabled).toBe(true);
        d.resolve({ ok: true });
        await waitFor(() => expect(button("Remove").disabled).toBe(false));
    });

    it("dates a removed file from the clock it is given", async () => {
        mount(task(), { detail: { media: [mediaItem(2, { removed_at: NOW - 180_000, removed_by: "operator" })] } });
        expect(await screen.findByText(/removed by operator, 3m ago/)).toBeTruthy();
    });
});

describe("KanbanDrawer: documents", () => {
    it("shows the document the task wrote, and opens it in the editor", () => {
        mount(task({}, { doc: "docs/specs/SPEC_X.md" }));
        expect(heading("Document")).toBeTruthy();
        expect(document.querySelector(".kanban-drawer__doc")?.textContent).toBe("docs/specs/SPEC_X.md");
        fireEvent.click(button("Open"));
        expect(h.createBlock).toHaveBeenCalledWith({ meta: { view: "editor", file: "docs/specs/SPEC_X.md" } });
    });

    it("shows the spec a build starts after, labelled with the spec", () => {
        mount(task({}, { after: [after(3, { kind: "spec", doc: "docs/a.md" })] }));
        expect(document.querySelector(".kanban-drawer__doc")?.textContent).toBe("spec #3: docs/a.md");
    });

    it("counts research as well as spec, whatever its case", () => {
        mount(task({}, { after: [after(4, { kind: " Research ", doc: "docs/b.md" })] }));
        expect(document.querySelector(".kanban-drawer__doc")?.textContent).toBe(" Research  #4: docs/b.md");
    });

    it("lists its own document first, then each spec's", () => {
        mount(
            task(
                {},
                {
                    doc: "own.md",
                    after: [after(3, { kind: "spec", doc: "a.md" }), after(4, { kind: "research", doc: "b.md" })],
                }
            )
        );
        expect([...document.querySelectorAll(".kanban-drawer__doc")].map((e) => e.textContent)).toEqual([
            "own.md",
            "spec #3: a.md",
            "research #4: b.md",
        ]);
        fireEvent.click(screen.getAllByRole("button", { name: "Open" })[2]);
        expect(h.createBlock).toHaveBeenCalledWith({ meta: { view: "editor", file: "b.md" } });
    });

    it("skips what starts first when it is no spec or wrote nothing", () => {
        mount(
            task(
                {},
                {
                    after: [
                        after(3, { kind: "build", doc: "build.md" }),
                        after(4, { kind: "spec" }),
                        after(5, { doc: "none.md" }),
                    ],
                }
            )
        );
        expect(heading("Document")).toBeNull();
    });

    it("has no document section when there is none", () => {
        mount(task());
        expect(heading("Document")).toBeNull();
    });
});

describe("KanbanDrawer: the result", () => {
    it("shows a finished task's result", () => {
        mount(task({ state: "done", result: "all merged" }));
        expect(heading("Result")).toBeTruthy();
        expect(screen.getByText("all merged")).toBeTruthy();
    });

    it("says why a failed or waiting task was last handed back", () => {
        mount(task({ state: "failed", result: "agent crashed" }));
        expect(heading("Last handed back because")).toBeTruthy();
        expect(heading("Result")).toBeNull();
        cleanup();
        mount(task({ state: "open", result: "handed back" }));
        expect(heading("Last handed back because")).toBeTruthy();
    });

    it("shows no result section without a result", () => {
        mount(task({ state: "done" }));
        expect(heading("Result")).toBeNull();
        expect(heading("Last handed back because")).toBeNull();
    });
});

describe("KanbanDrawer: taking it back", () => {
    it("offers it for a task an agent holds, or one blocked on a question", () => {
        mount(claimed());
        expect(button("Take it back")).toBeTruthy();
        cleanup();
        mount(task({ state: "blocked", claimed_by: "Naki-2", holder: naki }));
        expect(button("Take it back")).toBeTruthy();
    });

    it("offers it for no other state", () => {
        for (const state of ["open", "held", "review", "done", "failed", "cancelled"] as const) {
            mount(task({ state }));
            expect(screen.queryByRole("button", { name: "Take it back" })).toBeNull();
            expect(screen.queryByLabelText("Agent to give it to")).toBeNull();
            cleanup();
        }
    });

    it("explains an agent that cannot work, by name or as its agent", () => {
        mount(claimed({ holder_unavailable: { state: "limited", title: "x", since_ms: 1 } }));
        expect(
            screen.getByText("Naki-2 can't work right now. Take the task back to let someone else do it.")
        ).toBeTruthy();
        cleanup();
        mount(claimed({ claimed_by: "", holder_unavailable: { state: "signed_out", title: "x", since_ms: 1 } }));
        expect(
            screen.getByText("Its agent can't work right now. Take the task back to let someone else do it.")
        ).toBeTruthy();
    });

    it("says nothing of that while the agent can work", () => {
        mount(claimed());
        expect(screen.queryByText(/can't work right now/)).toBeNull();
    });

    it("sends the release and keeps no confirmation when srv does it at once", async () => {
        const { act } = mount(claimed(), { answers: [{ ok: true }] });
        fireEvent.click(button("Take it back"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "release" });
        expect(document.querySelector(".kanban-drawer__confirm")).toBeNull();
    });

    it("shows srv's warning as an alert and asks again with the yes", async () => {
        const { act } = mount(claimed(), { answers: [{ ok: false, confirm: "It has a PR. Sure?" }, { ok: true }] });
        fireEvent.click(button("Take it back"));
        const alert = await screen.findByRole("alert");
        expect(alert.textContent).toContain("It has a PR. Sure?");
        expect(alert.querySelector("p")?.className).toBe("kanban-drawer__reason");
        fireEvent.click(button("Take it back anyway"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(2));
        expect(act.mock.calls[1][0]).toEqual({ id: "w1", action: "release", confirm: true });
        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    });

    it("keeps no warning for a refusal that carries none, or for no answer", async () => {
        const { act } = mount(claimed(), { answers: [{ ok: false }, null] });
        fireEvent.click(button("Take it back"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(screen.queryByRole("alert")).toBeNull();
        fireEvent.click(button("Take it back"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(2));
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("drops an old warning when the next answer carries none", async () => {
        const { act } = mount(claimed(), { answers: [{ ok: false, confirm: "Sure?" }, { ok: false }] });
        fireEvent.click(button("Take it back"));
        await screen.findByRole("alert");
        fireEvent.click(button("Take it back"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(2));
        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    });

    it("drops the warning on Leave it without a second request", async () => {
        const { act } = mount(claimed(), { answers: [{ ok: false, confirm: "Sure?" }] });
        fireEvent.click(button("Take it back"));
        await screen.findByRole("alert");
        fireEvent.click(button("Leave it"));
        expect(screen.queryByRole("alert")).toBeNull();
        expect(act).toHaveBeenCalledTimes(1);
    });

    it("keeps the warning's buttons off while a move is in flight", async () => {
        const d = deferred<KanbanActResult | null>();
        const { act } = mount(claimed(), { answers: [{ ok: false, confirm: "Sure?" }] });
        await Promise.resolve();
        fireEvent.click(button("Take it back"));
        await screen.findByRole("alert");
        act.mockReturnValueOnce(d.promise);
        fireEvent.click(button("Take it back anyway"));
        await waitFor(() => expect(button("Take it back anyway").disabled).toBe(true));
        expect(button("Leave it").disabled).toBe(true);
        d.resolve({ ok: true });
        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    });

    it("keeps Take it back off while a move is in flight", async () => {
        const d = deferred<KanbanActResult | null>();
        const { act } = mount(claimed());
        act.mockReturnValueOnce(d.promise);
        expect(button("Take it back").disabled).toBe(false);
        fireEvent.click(button("Take it back"));
        await waitFor(() => expect(button("Take it back").disabled).toBe(true));
        d.resolve({ ok: true });
        await waitFor(() => expect(button("Take it back").disabled).toBe(false));
    });

    it("warns again on a repeated take-back that still needs a yes", async () => {
        const { act } = mount(claimed(), {
            answers: [
                { ok: false, confirm: "First?" },
                { ok: false, confirm: "Second?" },
            ],
        });
        fireEvent.click(button("Take it back"));
        await screen.findByText("First?");
        fireEvent.click(button("Take it back anyway"));
        await screen.findByText("Second?");
        expect(act).toHaveBeenCalledTimes(2);
    });
});

describe("KanbanDrawer: giving it to someone else", () => {
    it("offers Give it to…, Anyone, then the running agents the task is not already with", () => {
        mount(claimed(), { agents: [naki, loap, clare, gone, stopped] });
        // The agent order is the recency order (reversed here), without the holder, the stopped and the gone.
        expect(optionLabels("Agent to give it to")).toEqual([
            "Give it to…",
            "Anyone",
            "Clare (waiting on a question)",
            "Loap-3 (idle)",
        ]);
    });

    it("keeps Give off until an agent is chosen, then sends the reassignment", async () => {
        const { act } = mount(claimed());
        expect(button("Give").disabled).toBe(true);
        fireEvent.change(screen.getByLabelText("Agent to give it to"), { target: { value: "Loap-3" } });
        expect(button("Give").disabled).toBe(false);
        fireEvent.click(button("Give"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "reassign", agent: "Loap-3" });
    });

    it("sends Anyone as no agent", async () => {
        const { act } = mount(claimed());
        fireEvent.change(screen.getByLabelText("Agent to give it to"), { target: { value: "__anyone" } });
        fireEvent.click(button("Give"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "reassign", agent: "" });
    });

    it("clears the choice once the task is given", async () => {
        const { act } = mount(claimed());
        const select = screen.getByLabelText("Agent to give it to") as HTMLSelectElement;
        fireEvent.change(select, { target: { value: "Loap-3" } });
        fireEvent.click(button("Give"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        await waitFor(() => expect(button("Give").disabled).toBe(true));
    });

    it("keeps the choice when srv asks first or refuses", async () => {
        const { act } = mount(claimed(), { answers: [{ ok: false, confirm: "Sure?" }, null] });
        fireEvent.change(screen.getByLabelText("Agent to give it to"), { target: { value: "Loap-3" } });
        fireEvent.click(button("Give"));
        await screen.findByRole("alert");
        expect(button("Give").disabled).toBe(false);
        fireEvent.click(button("Leave it"));
        fireEvent.click(button("Give"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(2));
        expect(button("Give").disabled).toBe(false);
    });

    it("repeats the reassignment with the yes", async () => {
        const { act } = mount(claimed(), { answers: [{ ok: false, confirm: "Sure?" }, { ok: true }] });
        fireEvent.change(screen.getByLabelText("Agent to give it to"), { target: { value: "Loap-3" } });
        fireEvent.click(button("Give"));
        await screen.findByRole("alert");
        fireEvent.click(button("Take it back anyway"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(2));
        expect(act.mock.calls[1][0]).toEqual({ id: "w1", action: "reassign", agent: "Loap-3", confirm: true });
    });

    it("keeps Give off while a move is in flight", async () => {
        const d = deferred<KanbanActResult | null>();
        const { act } = mount(claimed());
        act.mockReturnValueOnce(d.promise);
        fireEvent.change(screen.getByLabelText("Agent to give it to"), { target: { value: "Loap-3" } });
        fireEvent.click(button("Take it back"));
        await waitFor(() => expect(button("Give").disabled).toBe(true));
        d.resolve({ ok: true });
        await waitFor(() => expect(button("Give").disabled).toBe(false));
    });
});

describe("KanbanDrawer: starting a waiting task", () => {
    it("offers it for a waiting task, in the backlog", () => {
        mount(task());
        expect(screen.getByLabelText("Agent to start it on")).toBeTruthy();
        expect(button("Start")).toBeTruthy();
    });

    it("offers it for a task that is spec'd, and for one on hold", () => {
        mount(task({}, { after: [after(3, { kind: "spec", state: "done" })] }));
        expect(button("Start")).toBeTruthy();
        cleanup();
        mount(task({ state: "held" }));
        expect(button("Start")).toBeTruthy();
    });

    it("offers it for no other column", () => {
        for (const t of [
            claimed(),
            task({ state: "review" }),
            task({ state: "done" }),
            task({ state: "failed" }),
            task({ state: "blocked" }),
        ]) {
            mount(t);
            expect(screen.queryByLabelText("Agent to start it on")).toBeNull();
            expect(screen.queryByRole("button", { name: "Start" })).toBeNull();
            cleanup();
        }
    });

    it("lists the running agents in recency order, with their states", () => {
        mount(task(), { agents: [naki, loap, clare, gone, stopped] });
        expect(optionLabels("Agent to start it on")).toEqual([
            "Start on…",
            "Clare (waiting on a question)",
            "Loap-3 (idle)",
            "Naki-2 (working)",
        ]);
    });

    it("keeps Start off until an agent is chosen, then starts it there", async () => {
        const { act } = mount(task());
        expect(button("Start").disabled).toBe(true);
        fireEvent.change(screen.getByLabelText("Agent to start it on"), { target: { value: "Naki-2" } });
        expect(button("Start").disabled).toBe(false);
        fireEvent.click(button("Start"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "start", agent: "Naki-2" });
    });

    it("keeps Start and Hold off while a move is in flight", async () => {
        const d = deferred<KanbanActResult | null>();
        const { act } = mount(task());
        act.mockReturnValueOnce(d.promise);
        fireEvent.change(screen.getByLabelText("Agent to start it on"), { target: { value: "Naki-2" } });
        fireEvent.click(button("Hold"));
        await waitFor(() => expect(button("Hold").disabled).toBe(true));
        expect(button("Start").disabled).toBe(true);
        d.resolve({ ok: true });
        await waitFor(() => expect(button("Hold").disabled).toBe(false));
        expect(button("Start").disabled).toBe(false);
    });

    it("holds a waiting task", async () => {
        const { act } = mount(task());
        fireEvent.click(button("Hold"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "move", hold: true });
    });

    it("releases a task on hold", async () => {
        const { act } = mount(task({ state: "held" }));
        expect(screen.queryByRole("button", { name: "Hold" })).toBeNull();
        fireEvent.click(button("Release hold"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "move", hold: false });
    });

    it("shows the hold button's icon for what it does", () => {
        mount(task());
        expect(button("Hold").querySelector("i, svg")?.className.toString()).toContain("pause");
        cleanup();
        mount(task({ state: "held" }));
        expect(button("Release hold").querySelector("i, svg")?.className.toString()).toContain("play");
    });

    it("lets a task aimed at one agent go to anyone", async () => {
        const { act } = mount(task({ target_agent: "Loap-3" }));
        fireEvent.click(button("Give to anyone"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "move", agent: "" });
    });

    it("offers no Give to anyone for a task aimed at nobody", () => {
        mount(task());
        expect(screen.queryByRole("button", { name: "Give to anyone" })).toBeNull();
    });
});

describe("KanbanDrawer: answering and sending back", () => {
    it("asks for an answer on a blocked task and sends it", async () => {
        const { act } = mount(task({ state: "blocked", claimed_by: "Naki-2", holder: naki }));
        const box = screen.getByPlaceholderText("Your answer") as HTMLTextAreaElement;
        expect(box.rows).toBe(3);
        expect(box.className).toContain("kanban-drawer__text");
        expect(button("Answer").disabled).toBe(true);
        fireEvent.input(box, { target: { value: "  use main  " } });
        expect(button("Answer").disabled).toBe(false);
        fireEvent.click(button("Answer"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        // The answer goes as typed, not trimmed.
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "answer", text: "  use main  " });
        await waitFor(() => expect(box.value).toBe(""));
    });

    it("keeps Answer off for an answer of spaces", () => {
        mount(task({ state: "blocked", claimed_by: "Naki-2", holder: naki }));
        fireEvent.input(screen.getByPlaceholderText("Your answer"), { target: { value: "   " } });
        expect(button("Answer").disabled).toBe(true);
    });

    it("keeps Answer off while a move is in flight", async () => {
        const d = deferred<KanbanActResult | null>();
        const { act } = mount(task({ state: "blocked", claimed_by: "Naki-2", holder: naki }));
        act.mockReturnValueOnce(d.promise);
        fireEvent.input(screen.getByPlaceholderText("Your answer"), { target: { value: "yes" } });
        fireEvent.click(button("Answer"));
        await waitFor(() => expect(button("Answer").disabled).toBe(true));
        // Disabled for the move, not for an empty box: the typed words are still there.
        expect((screen.getByPlaceholderText("Your answer") as HTMLTextAreaElement).value).toBe("yes");
        d.resolve({ ok: true });
        await waitFor(() => expect((screen.getByPlaceholderText("Your answer") as HTMLTextAreaElement).value).toBe(""));
    });

    it("keeps what was typed when srv does not take the move", async () => {
        const { act } = mount(task({ state: "blocked", claimed_by: "Naki-2", holder: naki }), { answers: [null] });
        const box = screen.getByPlaceholderText("Your answer") as HTMLTextAreaElement;
        fireEvent.input(box, { target: { value: "yes" } });
        fireEvent.click(button("Answer"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        await waitFor(() => expect(button("Answer").disabled).toBe(false));
        expect(box.value).toBe("yes");
    });

    it("clears what was typed once any move is taken", async () => {
        const { act } = mount(task({ state: "review" }));
        const box = screen.getByPlaceholderText("What to fix (for Send back)") as HTMLTextAreaElement;
        fireEvent.input(box, { target: { value: "more tests" } });
        fireEvent.click(button("Send back"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "send_back", text: "more tests" });
        await waitFor(() => expect(box.value).toBe(""));
    });

    it("has a box only for a blocked task or one in review", () => {
        for (const t of [
            task(),
            claimed(),
            task({ state: "done" }),
            task({ state: "failed" }),
            task({ state: "held" }),
        ]) {
            mount(t);
            expect(actions().querySelector("textarea")).toBeNull();
            cleanup();
        }
    });

    it("offers Answer only when blocked", () => {
        mount(task({ state: "review" }));
        expect(screen.queryByRole("button", { name: "Answer" })).toBeNull();
    });
});

describe("KanbanDrawer: accepting", () => {
    it("accepts a task in review", async () => {
        const { act } = mount(task({ state: "review" }));
        expect(button("Accept").title).toBe("");
        fireEvent.click(button("Accept"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "accept" });
    });

    it("names the kind of document being accepted, whatever its case or spacing", () => {
        mount(task({ state: "review", kind: "spec" }));
        expect(button("Accept spec")).toBeTruthy();
        cleanup();
        mount(task({ state: "review", kind: "  Research " }));
        expect(button("Accept research")).toBeTruthy();
        cleanup();
        mount(task({ state: "review", kind: "build" }));
        expect(button("Accept")).toBeTruthy();
    });

    it("says what accepting a spec unlocks", () => {
        mount(
            task(
                { state: "review", kind: "spec" },
                {
                    followers: [
                        after(8, { title: "Build: X", state: "open" }),
                        after(9, { title: "Build: Y", state: "held" }),
                        after(10, { title: "Build: Z", state: "done" }),
                    ],
                }
            )
        );
        expect(button("Accept spec").title).toBe('Unlocks #8 "Build: X"; Unlocks #9 "Build: Y"');
    });

    it("says nothing of unlocking when nothing waits, or when it is no spec", () => {
        mount(task({ state: "review", kind: "spec" }, { followers: [after(10, { state: "done" })] }));
        expect(button("Accept spec").title).toBe("");
        cleanup();
        mount(task({ state: "review", kind: "build" }, { followers: [after(8)] }));
        expect(button("Accept").title).toBe("");
    });

    it("keeps Accept and Send back off while a move is in flight", async () => {
        const d = deferred<KanbanActResult | null>();
        const { act } = mount(task({ state: "review" }));
        act.mockReturnValueOnce(d.promise);
        fireEvent.click(button("Accept"));
        await waitFor(() => expect(button("Accept").disabled).toBe(true));
        expect(button("Send back").disabled).toBe(true);
        d.resolve({ ok: true });
        await waitFor(() => expect(button("Send back").disabled).toBe(false));
    });

    it("offers neither outside review", () => {
        mount(task());
        expect(screen.queryByRole("button", { name: /^Accept/ })).toBeNull();
        expect(screen.queryByRole("button", { name: "Send back" })).toBeNull();
    });

    it("reopens what an agent accepted, carrying what was typed", async () => {
        const { act } = mount(task({ state: "done", accepted_by: { name: "AgentY", said: "ok", at: 5 } }));
        expect(button("Reopen").title).toBe("AgentY accepted it. This puts it back in Review.");
        fireEvent.click(button("Reopen"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "reopen", text: "" });
    });

    it("keeps Reopen off while a move is in flight", async () => {
        const d = deferred<KanbanActResult | null>();
        const { act } = mount(task({ state: "done", accepted_by: { name: "AgentY", said: "ok", at: 5 } }));
        act.mockReturnValueOnce(d.promise);
        fireEvent.click(button("Reopen"));
        await waitFor(() => expect(button("Reopen").disabled).toBe(true));
        d.resolve({ ok: true });
        await waitFor(() => expect(button("Reopen").disabled).toBe(false));
    });
});

describe("KanbanDrawer: trying again and opening the agent", () => {
    it("retries a failed task", async () => {
        const { act } = mount(task({ state: "failed" }));
        fireEvent.click(button("Try again"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "retry" });
    });

    it("keeps Try again off while a move is in flight", async () => {
        const d = deferred<KanbanActResult | null>();
        const { act } = mount(task({ state: "failed" }));
        act.mockReturnValueOnce(d.promise);
        fireEvent.click(button("Try again"));
        await waitFor(() => expect(button("Try again").disabled).toBe(true));
        d.resolve({ ok: true });
        await waitFor(() => expect(button("Try again").disabled).toBe(false));
    });

    it("offers it for no other state", () => {
        mount(claimed());
        expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
    });

    it("opens the agent holding the task", () => {
        const { onOpenAgent } = mount(claimed());
        fireEvent.click(button("Open Naki-2"));
        expect(onOpenAgent).toHaveBeenCalledWith("b-n");
    });

    it("offers no agent to open when none holds it", () => {
        mount(task());
        expect(screen.queryByRole("button", { name: /^Open / })).toBeNull();
    });
});

describe("KanbanDrawer: cancelling", () => {
    it("cancels a task with the text typed", async () => {
        const { act } = mount(task({ state: "review" }));
        fireEvent.input(screen.getByPlaceholderText("What to fix (for Send back)"), {
            target: { value: "not needed" },
        });
        fireEvent.click(button("Cancel task"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "cancel", text: "not needed", followers: undefined });
        expect("followers" in (lastCall(act) as object)).toBe(true);
    });

    it("keeps Cancel task off while a move is in flight", async () => {
        const d = deferred<KanbanActResult | null>();
        const { act } = mount(task());
        act.mockReturnValueOnce(d.promise);
        fireEvent.click(button("Cancel task"));
        await waitFor(() => expect(button("Cancel task").disabled).toBe(true));
        d.resolve({ ok: true });
        await waitFor(() => expect(button("Cancel task").disabled).toBe(false));
    });

    it("is offered until the task is finished", () => {
        for (const t of [
            task(),
            claimed(),
            task({ state: "review" }),
            task({ state: "failed" }),
            task({ state: "blocked" }),
        ]) {
            mount(t);
            expect(button("Cancel task")).toBeTruthy();
            cleanup();
        }
        for (const t of [
            task({ state: "done" }),
            task({ state: "cancelled" }),
            task({ state: "done" }, { pr: { released_in: "v1" } }),
        ]) {
            mount(t);
            expect(screen.queryByRole("button", { name: "Cancel task" })).toBeNull();
            cleanup();
        }
    });

    it("asks what becomes of the builds waiting on a spec, and cancels them by default", async () => {
        const { act } = mount(
            task({ kind: "spec" }, { followers: [after(8), after(9, { state: "held" }), after(10, { state: "done" })] })
        );
        const box = screen.getByRole("checkbox", { name: /Cancelling also cancels/ }) as HTMLInputElement;
        expect(box.checked).toBe(true);
        expect(box.closest("label")?.textContent).toContain("Cancelling also cancels #8, #9 (else they go on hold)");
        fireEvent.click(button("Cancel task"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "cancel", text: "", followers: "cancel" });
    });

    it("holds the builds when the tick is taken off", async () => {
        const { act } = mount(task({ kind: "research" }, { followers: [after(8)] }));
        fireEvent.click(screen.getByRole("checkbox", { name: /Cancelling also cancels/ }));
        expect((screen.getByRole("checkbox", { name: /Cancelling also cancels/ }) as HTMLInputElement).checked).toBe(
            false
        );
        fireEvent.click(button("Cancel task"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect((act.mock.calls[0][0] as KanbanActRequest).followers).toBe("hold");
        // Ticked again, it cancels.
        fireEvent.click(screen.getByRole("checkbox", { name: /Cancelling also cancels/ }));
        fireEvent.click(button("Cancel task"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(2));
        expect((act.mock.calls[1][0] as KanbanActRequest).followers).toBe("cancel");
    });

    it("asks nothing when only finished tasks wait on it", () => {
        mount(task({ kind: "spec" }, { followers: [after(10, { state: "done" }), after(11, { state: "cancelled" })] }));
        expect(screen.queryByRole("checkbox", { name: /Cancelling also cancels/ })).toBeNull();
    });

    it("asks nothing for a task that is no spec", () => {
        mount(task({ kind: "build" }, { followers: [after(8)] }));
        expect(screen.queryByRole("checkbox", { name: /Cancelling also cancels/ })).toBeNull();
    });

    it("sends no followers choice for a task nothing waits on", async () => {
        const { act } = mount(task({ kind: "spec" }));
        fireEvent.click(button("Cancel task"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect((act.mock.calls[0][0] as KanbanActRequest).followers).toBeUndefined();
    });

    it("asks nothing once the task is finished", () => {
        mount(task({ kind: "spec", state: "done" }, { followers: [after(8)] }));
        expect(screen.queryByRole("checkbox", { name: /Cancelling also cancels/ })).toBeNull();
    });
});

describe("KanbanDrawer: a move's bookkeeping", () => {
    it("sends every move with the task's id", async () => {
        const { act } = mount(task({ id: "w77", state: "failed" }));
        fireEvent.click(button("Try again"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect((act.mock.calls[0][0] as KanbanActRequest).id).toBe("w77");
    });

    it("lets the move's own fields win over nothing: the id is the task's", async () => {
        const { act } = mount(task({ state: "review" }));
        fireEvent.click(button("Send back"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect(Object.keys(act.mock.calls[0][0]).sort()).toEqual(["action", "id", "text"]);
    });
});

describe("KanbanDrawer: the history", () => {
    it("says Loading… until the history arrives", async () => {
        const d = deferred<KanbanTaskDetail>();
        mount(task(), { detail: d.promise });
        expect(heading("History")).toBeTruthy();
        expect(screen.getByText("Loading…")).toBeTruthy();
        expect(document.querySelector(".kanban-history")).toBeNull();
        d.resolve({ task: task(), events: [event(1)] });
        await screen.findByText(/took it/);
        expect(screen.queryByText("Loading…")).toBeNull();
    });

    it("lists each event with its age, who did it and what", async () => {
        mount(task(), {
            detail: {
                events: [
                    event(1, { at: NOW - 30_000, actor: "Naki-2", kind: "claimed" }),
                    event(2, { at: NOW - 7_200_000, actor: "", actor_kind: "operator", kind: "created" }),
                    event(3, { at: NOW - 61_000, actor: "", actor_kind: "system", kind: "expired" }),
                ],
            },
        });
        await screen.findByText(/Naki-2 took it/);
        const lines = [...document.querySelectorAll(".kanban-history__line")];
        expect(lines.map((l) => l.querySelector(".kanban-history__age")?.textContent)).toEqual(["30s", "2h", "1m"]);
        expect(lines.map((l) => l.querySelector("span:not(.kanban-history__age)")?.textContent?.trim())).toEqual([
            "Naki-2 took it",
            "You created it",
            "AgentMux lost its lease",
        ]);
    });

    it("never shows a negative age", async () => {
        mount(task(), { detail: { events: [event(1, { at: NOW + 90_000 })] } });
        await screen.findByText(/took it/);
        expect(document.querySelector(".kanban-history__age")?.textContent).toBe("0s");
    });

    it("words each kind of event", async () => {
        const kinds = {
            created: "created it",
            filed_for_later: "filed it for later",
            claimed: "took it",
            started: "started it",
            held: "put it on hold",
            unheld: "released it from hold",
            assigned: "moved it",
            edited: "edited it",
            updated: "updated it",
            asked: "asked",
            answered: "answered",
            submitted: "handed it in",
            completed: "finished it",
            accepted: "accepted it",
            reopened: "took the acceptance back",
            sent_back: "sent it back",
            released: "handed it back",
            expired: "lost its lease",
            cancelled: "cancelled it",
            retried: "tried it again",
            linked: "linked",
            reminded: "reminded its agent",
            tracked: "started following it",
            taken_back: "took it back",
            reassigned: "took it back and gave it to someone else",
        };
        mount(task(), { detail: { events: Object.keys(kinds).map((kind, i) => event(i, { kind })) } });
        await screen.findByText(/Naki-2 created it/);
        const words = [...document.querySelectorAll(".kanban-history__line")].map((l) =>
            l.querySelector("span:not(.kanban-history__age)")?.textContent?.trim()
        );
        expect(words).toEqual(Object.values(kinds).map((w) => `Naki-2 ${w}`));
    });

    it("reads an event kind this build does not word as itself", async () => {
        mount(task(), { detail: { events: [event(1, { kind: "teleported" })] } });
        expect(await screen.findByText(/Naki-2 teleported/)).toBeTruthy();
    });

    it("adds what the event said, after a colon", async () => {
        mount(task(), {
            detail: { events: [event(1, { kind: "asked", body: "which repo?" }), event(2, { kind: "claimed" })] },
        });
        await screen.findByText(/Naki-2 asked/);
        const bodies = [...document.querySelectorAll(".kanban-history__body")];
        expect(bodies.map((b) => b.textContent)).toEqual([": which repo?"]);
    });

    it("shows an empty list for a task with no events yet", async () => {
        mount(task(), { detail: { events: [] } });
        await waitFor(() => expect(document.querySelector("ul.kanban-history")).toBeTruthy());
        expect(document.querySelectorAll(".kanban-history__line")).toHaveLength(0);
        expect(screen.queryByText("Loading…")).toBeNull();
    });

    it("asks for the history of the task shown, once", async () => {
        const { detail } = mount(task({ id: "w9" }));
        await waitFor(() => expect(detail).toHaveBeenCalledTimes(1));
        expect(detail).toHaveBeenCalledWith("w9");
    });

    it("does not ask again for an equal task after a refresh", async () => {
        const { detail, setTask } = mount(task());
        await waitFor(() => expect(detail).toHaveBeenCalledTimes(1));
        setTask(task({ title: "Same task, new object", payload: "changed" }));
        await Promise.resolve();
        expect(detail).toHaveBeenCalledTimes(1);
    });

    it("asks again when the task changes on the board", async () => {
        const { detail, setTask } = mount(task());
        await waitFor(() => expect(detail).toHaveBeenCalledTimes(1));
        setTask(task({ updated_at: 2 }));
        await waitFor(() => expect(detail).toHaveBeenCalledTimes(2));
        setTask(task({ updated_at: 2 }, { updated_at: 5 }));
        await waitFor(() => expect(detail).toHaveBeenCalledTimes(3));
    });

    it("asks for the new task's history when another task is opened", async () => {
        const { detail, setTask } = mount(task({ id: "w1" }));
        await waitFor(() => expect(detail).toHaveBeenCalledWith("w1"));
        setTask(task({ id: "w2" }));
        await waitFor(() => expect(detail).toHaveBeenLastCalledWith("w2"));
    });
});

describe("KanbanDrawer: markup the styles and the toasts rely on", () => {
    it("carries the drawer class, and keeps toasts off the actions", () => {
        mount(task());
        expect(screen.getByLabelText("Task 12").classList.contains("kanban-drawer")).toBe(true);
        expect(actions().hasAttribute("data-toast-avoid")).toBe(true);
    });

    it("drops the kit's look from the answer box", () => {
        mount(task({ state: "blocked", claimed_by: "Naki-2", holder: naki }));
        const box = screen.getByPlaceholderText("Your answer");
        expect(box.className).toBe("kanban-drawer__text");
    });

    it("gives each button its icon", () => {
        mount(
            claimed(
                { state: "blocked", accepted_by: { name: "AgentY", said: "", at: 1 } },
                { pr_url: "https://x/pull/1", doc: "a.md" }
            )
        );
        const icon = (name: string | RegExp) => button(name).querySelector("i")?.className;
        expect(icon("Open the PR")).toBe("fa-solid fa-code-pull-request");
        expect(icon("Open")).toBe("fa-solid fa-file-lines");
        expect(icon("Take it back")).toBe("fa-solid fa-rotate-left");
        expect(icon("Give")).toBe("fa-solid fa-share");
        expect(icon("Answer")).toBe("fa-solid fa-reply");
        expect(icon("Reopen")).toBe("fa-solid fa-rotate-left");
        expect(icon("Open Naki-2")).toBe("fa-solid fa-arrow-up-right-from-square");
        expect(icon("Cancel task")).toBe("fa-solid fa-ban");
    });

    it("gives the waiting and review buttons their icons", () => {
        mount(task({ target_agent: "Loap-3" }));
        expect(button("Start").querySelector("i")?.className).toBe("fa-solid fa-play");
        cleanup();
        mount(task({ state: "review" }));
        expect(button("Accept").querySelector("i")?.className).toBe("fa-solid fa-check");
        expect(button("Send back").querySelector("i")?.className).toBe("fa-solid fa-rotate-left");
        cleanup();
        mount(task({ state: "failed" }));
        expect(button("Try again").querySelector("i")?.className).toBe("fa-solid fa-rotate-right");
    });

    it("explains Take it back in its tooltip", () => {
        mount(claimed());
        expect(button("Take it back").title).toBe("Return it to the backlog; its agent is told to stop");
    });

    it("sets no tooltip on Accept when nothing is unlocked", () => {
        mount(task({ state: "review", kind: "spec" }));
        expect(button("Accept spec").hasAttribute("title")).toBe(false);
        cleanup();
        mount(task({ state: "review", kind: "build" }, { followers: [after(8)] }));
        expect(button("Accept").hasAttribute("title")).toBe(false);
    });

    it("words the accept button for a spec exactly, whatever the kind's spacing", () => {
        mount(task({ state: "review", kind: "  Research " }));
        const label = button("Accept research").textContent?.trim();
        expect(label).toBe("Accept research");
    });

    it("labels the cancel tick with the hold class and keeps its choice across a reopen", async () => {
        const t = task({ kind: "spec" }, { followers: [after(8)] });
        const { setTask } = mount(t);
        const label = () =>
            screen.getByRole("checkbox", { name: /Cancelling also cancels/ }).closest("label") as HTMLElement;
        expect(label().className).toBe("kanban-new__hold");
        fireEvent.click(screen.getByRole("checkbox", { name: /Cancelling also cancels/ }));
        // The tick leaves when the task finishes and comes back when it is reopened: the choice stays.
        setTask(task({ kind: "spec", state: "done" }, { followers: [after(8)] }));
        expect(screen.queryByRole("checkbox", { name: /Cancelling also cancels/ })).toBeNull();
        setTask(t);
        expect((screen.getByRole("checkbox", { name: /Cancelling also cancels/ }) as HTMLInputElement).checked).toBe(
            false
        );
    });

    it("gives the pickers' placeholder choices an empty value", () => {
        mount(claimed({ claimed_by: "" }));
        const values = (label: string) =>
            [...(screen.getByLabelText(label) as HTMLSelectElement).options].map((o) => o.value);
        expect(values("Agent to give it to")).toEqual(["", "__anyone", "Loap-3", "Naki-2"]);
        cleanup();
        mount(task());
        expect(values("Agent to start it on")).toEqual(["", "Loap-3", "Naki-2"]);
    });
});

describe("KanbanDrawer: what a move's answer does", () => {
    it("clears the text for any answer, even a refusal that srv sent back", async () => {
        const { act } = mount(task({ state: "review" }), { answers: [{ ok: false }] });
        const box = screen.getByPlaceholderText("What to fix (for Send back)") as HTMLTextAreaElement;
        fireEvent.input(box, { target: { value: "  more tests  " } });
        fireEvent.click(button("Send back"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        // Sent as typed, spaces and all.
        expect(act.mock.calls[0][0]).toEqual({ id: "w1", action: "send_back", text: "  more tests  " });
        await waitFor(() => expect(box.value).toBe(""));
    });

    it("cancels with the text as typed", async () => {
        const { act } = mount(task({ state: "review" }));
        fireEvent.input(screen.getByPlaceholderText("What to fix (for Send back)"), {
            target: { value: "  not needed " },
        });
        fireEvent.click(button("Cancel task"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        expect((act.mock.calls[0][0] as KanbanActRequest).text).toBe("  not needed ");
    });

    it("shows no warning for an answer that is ok even if it carries words", async () => {
        const { act } = mount(claimed(), { answers: [{ ok: true, confirm: "stray" }] });
        fireEvent.click(button("Take it back"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(1));
        await waitFor(() => expect(button("Take it back").disabled).toBe(false));
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("drops an old warning when the next move gets no answer at all", async () => {
        const { act } = mount(claimed(), { answers: [{ ok: false, confirm: "Sure?" }, null] });
        fireEvent.click(button("Take it back"));
        await screen.findByRole("alert");
        fireEvent.click(button("Take it back"));
        await waitFor(() => expect(act).toHaveBeenCalledTimes(2));
        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    });

    it("keeps Give to anyone off while a move is in flight", async () => {
        const d = deferred<KanbanActResult | null>();
        const { act } = mount(task({ target_agent: "Loap-3" }));
        act.mockReturnValueOnce(d.promise);
        fireEvent.click(button("Give to anyone"));
        await waitFor(() => expect(button("Give to anyone").disabled).toBe(true));
        d.resolve({ ok: true });
        await waitFor(() => expect(button("Give to anyone").disabled).toBe(false));
    });

    it("offers a waiting task's start to every running agent, even the one it was last with", () => {
        mount(task({ claimed_by: "Naki-2" }));
        expect(
            [...(screen.getByLabelText("Agent to start it on") as HTMLSelectElement).options].map((o) => o.value)
        ).toEqual(["", "Loap-3", "Naki-2"]);
    });

    it("lists a one-item checklist", () => {
        mount(task({}, { checklist: [{ text: "only step", status: "pending" }] }));
        expect(heading("Checklist")).toBeTruthy();
        expect(screen.getByText("only step")).toBeTruthy();
    });
});

describe("KanbanDrawer: the order of its parts", () => {
    it("runs from the header to the history, in the order the operator reads it", async () => {
        mount(
            task(
                { state: "blocked", claimed_by: "Naki-2", holder: naki, result: "so far so good" },
                {
                    question: "which repo?",
                    request_text: "please fix it",
                    checklist: [{ text: "write it", status: "pending" }],
                    after: [after(5, { kind: "spec", doc: "docs/a.md" })],
                    doc: "own.md",
                }
            ),
            { detail: { media: [mediaItem(1)] } }
        );
        await screen.findByText("shot1.png");
        const parts = [
            ".ui-side-panel__head",
            ".kanban-drawer__facts",
            "p.kanban-drawer__reason",
            "blockquote.kanban-drawer__quote",
            ".kanban-drawer__payload",
            ".kanban-checklist",
            ".kanban-after",
            ".kanban-media__grid",
            ".kanban-drawer__doc",
            ".kanban-drawer__actions",
            ".kanban-history",
        ].map((sel) => document.querySelector(sel) as HTMLElement);
        parts.forEach((p, i) => expect(p, `part ${i}`).toBeTruthy());
        for (let i = 1; i < parts.length; i++) {
            expect(
                parts[i - 1].compareDocumentPosition(parts[i]) & Node.DOCUMENT_POSITION_FOLLOWING,
                `part ${i}`
            ).toBeTruthy();
        }
        // The task's result sits between its documents and its actions.
        const result = [...document.querySelectorAll(".kanban-drawer__payload")].find(
            (e) => e.textContent === "so far so good"
        )!;
        expect(parts[8].compareDocumentPosition(result) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
        expect(result.compareDocumentPosition(parts[9]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });

    it("puts the box for your words above the buttons that use them", () => {
        mount(task({ state: "blocked", claimed_by: "Naki-2", holder: naki }));
        const box = actions().querySelector("textarea") as HTMLElement;
        const answer = button("Answer");
        expect(box.compareDocumentPosition(answer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
        const takeBack = button("Take it back");
        expect(takeBack.compareDocumentPosition(box) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });
});
