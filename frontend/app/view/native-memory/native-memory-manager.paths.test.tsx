// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NativeMemoryManager: the paths the main suite leaves open — out-of-order
 * replies, which events refetch what, the open panes' windows, switching agent
 * with a file or an error open, the debounce and its cleanup, file order, and
 * what the create row is told. Pinned before the component was split into
 * smaller functions.
 */

import { WpsEvent } from "@/app/store/mps-events";
import type { AgentDefinition, NativeMemoryFileMeta } from "@/app/store/rpc-api";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const listAgentDefinitionsMock = vi.fn();
const nativeMemoryListMock = vi.fn();
const launchCostMock = vi.fn();
const openPanesMock = vi.fn();

const hub = vi.hoisted(() => ({
    handlers: new Map<string, Set<(e: unknown) => void>>(),
    blockMetas: {} as Record<string, Record<string, unknown>>,
}));

vi.mock("@/app/store/mps", () => {
    const subscribe = (...subs: Array<{ eventType: string; handler: (e: unknown) => void }>) => {
        for (const sub of subs) {
            if (!hub.handlers.has(sub.eventType)) hub.handlers.set(sub.eventType, new Set());
            hub.handlers.get(sub.eventType)!.add(sub.handler);
        }
        return () => {
            for (const sub of subs) hub.handlers.get(sub.eventType)?.delete(sub.handler);
        };
    };
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void) =>
        subscribe({ eventType, handler: (e: { data?: unknown } | null) => handler(e?.data ?? null, e) });
    return { muxEventSubscribe: vi.fn(subscribe), onMuxEvent };
});

// The open panes' block meta, by block id.
vi.mock("@/app/store/mos", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/mos")>()),
    makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
    getMuxObjectAtom: (oref: string) => () => ({ meta: hub.blockMetas[oref] }),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListAgentDefinitionsCommand: (...args: unknown[]) => listAgentDefinitionsMock(...args),
        NativeMemoryListCommand: (...args: unknown[]) => nativeMemoryListMock(...args),
        MemoryLaunchCostCommand: (...args: unknown[]) => launchCostMock(...args),
        AgentOpenPanesCommand: (...args: unknown[]) => openPanesMock(...args),
    },
}));

let historyPanelMountCount = 0;
let reportDirty: ((dirty: boolean) => void) | undefined;
vi.mock("./NativeMemoryFileView", () => ({
    NativeMemoryFileView: (props: {
        agentId: string;
        filename: string;
        refreshNonce: () => number;
        onDirtyChange?: (dirty: boolean) => void;
    }) => {
        historyPanelMountCount++;
        reportDirty = props.onDirtyChange;
        return (
            <div data-testid="history-panel" data-refresh-nonce={String(props.refreshNonce())}>
                {`${props.agentId}:${props.filename}`}
            </div>
        );
    },
}));

vi.mock("./MemoryAdoptionPanel", () => ({
    MemoryAdoptionPanel: (props: { agentId: string; agentName: string }) => (
        <div data-testid="adoption-panel">{`${props.agentId}|${props.agentName}`}</div>
    ),
}));
vi.mock("./MemoryClaimsPanel", () => ({
    MemoryClaimsPanel: (props: { agentId: string; agentName: string }) => (
        <div data-testid="claims-panel">{`${props.agentId}|${props.agentName}`}</div>
    ),
}));

type CreateProps = {
    agentId: string;
    filenames: string[];
    hasIndex: boolean;
    readOnly: boolean;
    listUnknown: boolean;
    onCreated: (filename: string, createdFor: string) => void;
};
let createProps: CreateProps | undefined;
vi.mock("./NativeMemoryCreate", () => ({
    NativeMemoryCreate: (props: CreateProps) => {
        createProps = props;
        return (
            <div
                data-testid="create"
                data-agent={props.agentId}
                data-filenames={props.filenames.join(",")}
                data-has-index={String(props.hasIndex)}
                data-read-only={String(props.readOnly)}
                data-list-unknown={String(props.listUnknown)}
            />
        );
    },
}));

import { makeTestHostApi } from "@/app/host/test-host";
import { NativeMemoryManager } from "./native-memory-manager";

const agent = (id: string, name: string): AgentDefinition =>
    ({ id, name, slug: id, provider: "claude" }) as AgentDefinition;

const file = (filename: string, over: Partial<NativeMemoryFileMeta> = {}): NativeMemoryFileMeta => ({
    filename,
    is_index: false,
    metadata_type: null,
    size_bytes: 512,
    tokens_est: 128,
    lines: 10,
    modified_at: Date.now() - 86_400_000,
    ...over,
});

const cost = (tokens: number) => ({
    agent_id: "a1",
    provider: "claude",
    startup_file: "CLAUDE.md",
    delivered_by_file: true,
    parts: [
        {
            kind: "operator_config",
            name: "Operator Config",
            tokens_est: tokens,
            size_bytes: tokens * 4,
            lines: 1,
            via: "startup_file",
        },
    ],
    total_tokens_est: tokens,
});

const deferred = <T,>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: Error) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
};

const fire = (eventType: string) => hub.handlers.get(eventType)?.forEach((h) => h({ event: eventType, data: {} }));
const handlerCount = (eventType: string) => hub.handlers.get(eventType)?.size ?? 0;
const changed = (id: string) => `agent:memory:changed:${id}`;

const cardOf = (name: string) => screen.getByText(name).closest(".memory-agent-card") as HTMLElement;
const tileNames = () =>
    Array.from(document.querySelectorAll<HTMLElement>(".memory-file-card")).map(
        (el) => el.getAttribute("data-filename") ?? ""
    );
const openAgentName = () => document.querySelector(".native-memory-manager-agent-name")?.textContent;
const listCallsFor = (id: string) => nativeMemoryListMock.mock.calls.filter((c) => c[1]?.agent_id === id).length;
const launchCallsFor = (id: string) => launchCostMock.mock.calls.filter((c) => c[1]?.agent_id === id).length;
const flush = () => new Promise((r) => setTimeout(r, 0));

function paneWith(initial: string | null) {
    const [value, setValue] = createSignal<string | null>(initial);
    const setMeta = vi.fn();
    return {
        setValue,
        setMeta,
        pane: {
            meta: () => (value() === null ? {} : { "memory:agent": value() }) as MetaType,
            setMeta,
        },
    };
}

/** The agent list as `agents:changed` would bring it. */
async function setAgents(list: AgentDefinition[]) {
    listAgentDefinitionsMock.mockResolvedValue(list);
    fire(WpsEvent.AgentsChanged);
    await flush();
}

afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

beforeEach(() => {
    window.api = makeTestHostApi();
    listAgentDefinitionsMock.mockReset().mockResolvedValue([agent("a1", "Manoz"), agent("a2", "AgentY")]);
    nativeMemoryListMock.mockReset().mockResolvedValue({ files: [] });
    launchCostMock.mockReset().mockRejectedValue(new Error("no srv"));
    openPanesMock.mockReset().mockResolvedValue({ panes: [] });
    localStorage.clear();
    hub.handlers.clear();
    hub.blockMetas = {};
    historyPanelMountCount = 0;
    reportDirty = undefined;
    createProps = undefined;
});

describe("NativeMemoryManager — file distribution line", () => {
    test.each([
        [4, false],
        [5, true],
    ])("with %i files the distribution line shows: %s", async (n, shown) => {
        nativeMemoryListMock.mockResolvedValue({ files: Array.from({ length: n }, (_, i) => file(`f${i}.md`)) });
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz"));
        await waitFor(() => expect(tileNames()).toHaveLength(n));
        expect(!!screen.queryByTestId("memory-file-distribution")).toBe(shown);
    });
});

describe("NativeMemoryManager — an agent's label", () => {
    test.each([
        ["", "zed-slug", "zed-slug"],
        ["", "", "a1"],
        ["Zed", "zed-slug", "Zed"],
    ])("name %j and slug %j read as %j", async (name, slug, shown) => {
        listAgentDefinitionsMock.mockResolvedValue([{ id: "a1", name, slug, provider: "claude" }]);
        render(() => <NativeMemoryManager />);
        fireEvent.click(
            await waitFor(() => {
                const el = document.querySelector<HTMLElement>(".memory-agent-card");
                if (!el) throw new Error("no card");
                return el;
            })
        );
        await waitFor(() => expect(openAgentName()).toBe(shown));
    });
});

describe("NativeMemoryManager — the pane asks for an agent", () => {
    test("opening the one the pane names does not write it back into the pane", async () => {
        const { pane, setMeta } = paneWith("a2");
        render(() => <NativeMemoryManager pane={pane} />);
        await waitFor(() => expect(openAgentName()).toBe("AgentY"));
        expect(setMeta).not.toHaveBeenCalled();
    });

    test("a declined discard keeps the agent and withdraws the pane's request; the same request later does not ask again", async () => {
        nativeMemoryListMock.mockResolvedValue({ files: [file("one.md")] });
        const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
        const { pane, setValue, setMeta } = paneWith("a1");
        render(() => <NativeMemoryManager pane={pane} />);
        await waitFor(() => expect(openAgentName()).toBe("Manoz"));
        fireEvent.click(
            await waitFor(() => {
                const el = document.querySelector<HTMLElement>('.memory-file-card[data-filename="one.md"]');
                if (!el) throw new Error("no tile");
                return el;
            })
        );
        reportDirty?.(true);

        setValue("a2");
        await flush();
        expect(confirm).toHaveBeenCalledTimes(1);
        expect(openAgentName()).toBe("Manoz");
        expect(screen.getByTestId("history-panel")).toHaveTextContent("a1:one.md");
        expect(setMeta).toHaveBeenLastCalledWith({ "memory:agent": "a1" });

        // The pane now names the agent that is already open: nothing to ask.
        setValue("a1");
        await flush();
        expect(confirm).toHaveBeenCalledTimes(1);
        expect(openAgentName()).toBe("Manoz");
    });

    test("moving to another agent closes the file that was open", async () => {
        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) =>
            Promise.resolve({ files: [file(req.agent_id === "a1" ? "one.md" : "two.md")] })
        );
        const { pane, setValue } = paneWith("a1");
        render(() => <NativeMemoryManager pane={pane} />);
        await waitFor(() => expect(openAgentName()).toBe("Manoz"));
        fireEvent.click(
            await waitFor(() => {
                const el = document.querySelector<HTMLElement>('.memory-file-card[data-filename="one.md"]');
                if (!el) throw new Error("no tile");
                return el;
            })
        );
        expect(screen.getByTestId("history-panel")).toHaveTextContent("a1:one.md");

        setValue("a2");
        await waitFor(() => expect(openAgentName()).toBe("AgentY"));
        await waitFor(() => expect(tileNames()).toEqual(["two.md"]));
        expect(screen.queryByTestId("history-panel")).toBeNull();
        // The view is rebuilt for each agent and file, never carried over from the last: it was mounted
        // for a1's file, then again for a2 with the old name before that was cleared.
        expect(historyPanelMountCount).toBe(2);
    });
});

describe("NativeMemoryManager — counts and launch costs that arrive out of order", () => {
    test("an older count reply does not overwrite a newer one, nor does an older failure", async () => {
        vi.useFakeTimers();
        render(() => <NativeMemoryManager />);
        await vi.waitFor(() => expect(screen.getAllByText("No memories yet")).toHaveLength(2));

        const first = deferred<{ files: NativeMemoryFileMeta[] }>();
        const second = deferred<{ files: NativeMemoryFileMeta[] }>();
        nativeMemoryListMock.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
        fire(changed("a1"));
        await vi.advanceTimersByTimeAsync(250);
        fire(changed("a1"));
        await vi.advanceTimersByTimeAsync(250);
        expect(nativeMemoryListMock).toHaveBeenCalledTimes(4);

        second.resolve({ files: [file("a.md"), file("b.md")] });
        await vi.waitFor(() => expect(cardOf("Manoz")).toHaveTextContent("2 files"));
        first.resolve({ files: [file("a.md")] });
        await vi.advanceTimersByTimeAsync(0);
        expect(cardOf("Manoz")).toHaveTextContent("2 files");

        // And a late failure of the older request.
        const third = deferred<{ files: NativeMemoryFileMeta[] }>();
        const fourth = deferred<{ files: NativeMemoryFileMeta[] }>();
        nativeMemoryListMock.mockReturnValueOnce(third.promise).mockReturnValueOnce(fourth.promise);
        fire(changed("a1"));
        await vi.advanceTimersByTimeAsync(250);
        fire(changed("a1"));
        await vi.advanceTimersByTimeAsync(250);
        fourth.resolve({ files: [file("a.md"), file("b.md"), file("c.md")] });
        await vi.waitFor(() => expect(cardOf("Manoz")).toHaveTextContent("3 files"));
        third.reject(new Error("late"));
        await vi.advanceTimersByTimeAsync(0);
        expect(cardOf("Manoz")).toHaveTextContent("3 files");
        expect(screen.queryByText("Couldn't read memories")).toBeNull();
    });

    test("an older launch-cost reply does not overwrite a newer one", async () => {
        listAgentDefinitionsMock.mockResolvedValue([agent("a1", "Manoz")]);
        launchCostMock.mockResolvedValue(cost(1200));
        render(() => <NativeMemoryManager />);
        expect(await screen.findByText("Starts with ~1.2k tok")).toBeInTheDocument();

        const older = deferred<ReturnType<typeof cost>>();
        const newer = deferred<ReturnType<typeof cost>>();
        launchCostMock.mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
        fire(WpsEvent.MemoriesChanged);
        fire(WpsEvent.MemoriesChanged);
        newer.resolve(cost(5200));
        expect(await screen.findByText("Starts with ~5.2k tok")).toBeInTheDocument();
        older.resolve(cost(4500));
        await flush();
        expect(screen.getByText("Starts with ~5.2k tok")).toBeInTheDocument();
        expect(screen.queryByText("Starts with ~4.5k tok")).toBeNull();
    });

    test.each([
        ["Global Memory", WpsEvent.MemoriesChanged],
        ["an agent's content", WpsEvent.AgentContentChanged],
        ["the skills", WpsEvent.SkillsChanged],
        ["an agent's skills", WpsEvent.AgentSkillsChanged],
        ["the agents", WpsEvent.AgentsChanged],
    ])("a change to %s refetches every agent's launch cost", async (_what, event) => {
        launchCostMock.mockResolvedValue(cost(1200));
        render(() => <NativeMemoryManager />);
        await waitFor(() => expect(screen.getAllByText("Starts with ~1.2k tok")).toHaveLength(2));

        launchCostMock.mockResolvedValue(cost(2400));
        fire(event);
        await waitFor(() => expect(screen.getAllByText("Starts with ~2.4k tok")).toHaveLength(2));
    });
});

describe("NativeMemoryManager — the open panes' windows", () => {
    const pane = (agentId: string, blockId: string) => ({ agent_id: agentId, block_id: blockId });
    const withWindow = (blockId: string) => {
        hub.blockMetas[`block:${blockId}`] = {
            "agent:context": { tokens: 46_000, model: "m", window: 200_000, windowSource: "reported", source: "live" },
        };
    };

    test("a pane's window goes to its own agent's card and no other", async () => {
        withWindow("b1");
        launchCostMock.mockResolvedValue(cost(5200));
        openPanesMock.mockResolvedValue({ panes: [pane("a2", "b1")] });
        render(() => <NativeMemoryManager />);
        await waitFor(() => expect(cardOf("AgentY")).toHaveTextContent(/\d+% of 200k/));
        expect(cardOf("Manoz")).toHaveTextContent("Starts with ~5.2k tok");
        expect(cardOf("Manoz")).not.toHaveTextContent("of 200k");
    });

    test("a card says what it takes of the window, and whether the agent's Personal Memory is delivered", async () => {
        withWindow("b1");
        nativeMemoryListMock.mockResolvedValue({ files: [file("notes.md")] });
        launchCostMock.mockResolvedValue({
            ...cost(5200),
            parts: [
                {
                    kind: "operator_config",
                    name: "Operator Config",
                    tokens_est: 5000,
                    size_bytes: 20000,
                    lines: 1,
                    via: "startup_file",
                },
            ],
        });
        listAgentDefinitionsMock.mockResolvedValue([agent("a1", "Manoz")]);
        openPanesMock.mockResolvedValue({ panes: [pane("a1", "b1")] });
        render(() => <NativeMemoryManager />);
        await waitFor(() => expect(cardOf("Manoz")).toHaveTextContent(/\d+% of 200k/));
        // Claude's launch carries no personal part here, so the card says so.
        expect(cardOf("Manoz")).toHaveTextContent("Not delivered");
    });

    test("a tracked-blocks change refetches the panes, and a slower older reply is dropped", async () => {
        withWindow("b1");
        launchCostMock.mockResolvedValue(cost(5200));
        listAgentDefinitionsMock.mockResolvedValue([agent("a1", "Manoz")]);
        // The first read (before the agents arrive) is the slow one.
        const older = deferred<{ panes: ReturnType<typeof pane>[] }>();
        openPanesMock.mockReturnValueOnce(older.promise).mockResolvedValue({ panes: [] });
        render(() => <NativeMemoryManager />);
        await screen.findByText("Starts with ~5.2k tok");
        const reads = openPanesMock.mock.calls.length;

        fire(WpsEvent.ProcessBrokerTrackedBlocksChanged);
        await waitFor(() => expect(openPanesMock).toHaveBeenCalledTimes(reads + 1));
        await flush();
        older.resolve({ panes: [pane("a1", "b1")] });
        await flush();
        expect(cardOf("Manoz")).not.toHaveTextContent("of 200k");
    });

    test("coming back to the window refetches the panes and the launch costs; a closed pane no longer does", async () => {
        launchCostMock.mockResolvedValue(cost(1200));
        render(() => <NativeMemoryManager />);
        await waitFor(() => expect(screen.getAllByText("Starts with ~1.2k tok")).toHaveLength(2));
        const panes = openPanesMock.mock.calls.length;
        const costs = launchCostMock.mock.calls.length;

        window.dispatchEvent(new Event("focus"));
        await waitFor(() => expect(openPanesMock).toHaveBeenCalledTimes(panes + 1));
        await waitFor(() => expect(launchCostMock).toHaveBeenCalledTimes(costs + 2));

        cleanup();
        window.dispatchEvent(new Event("focus"));
        await flush();
        expect(openPanesMock).toHaveBeenCalledTimes(panes + 1);
        expect(launchCostMock).toHaveBeenCalledTimes(costs + 2);
    });
});

describe("NativeMemoryManager — the set of agents changes", () => {
    test("a reordered list changes nothing; a removed agent is dropped and, back again, is read afresh", async () => {
        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) =>
            Promise.resolve({
                files:
                    req.agent_id === "a2" ? [file("1.md"), file("2.md"), file("3.md"), file("4.md"), file("5.md")] : [],
            })
        );
        launchCostMock.mockResolvedValue(cost(1200));
        render(() => <NativeMemoryManager />);
        await waitFor(() => expect(cardOf("AgentY")).toHaveTextContent("5 files"));
        await waitFor(() => expect(screen.getAllByText("Starts with ~1.2k tok")).toHaveLength(2));
        const panes = openPanesMock.mock.calls.length;

        // The same agents in another order: no new fetch round.
        await setAgents([agent("a2", "AgentY"), agent("a1", "Manoz")]);
        expect(openPanesMock).toHaveBeenCalledTimes(panes);
        expect(listCallsFor("a1")).toBe(1);

        // One leaves: the panes are read again; the other is not re-read.
        await setAgents([agent("a1", "Manoz")]);
        await waitFor(() => expect(openPanesMock).toHaveBeenCalledTimes(panes + 1));
        expect(listCallsFor("a1")).toBe(1);
        expect(screen.queryByText("AgentY")).toBeNull();

        // It returns with a different folder, and its launch cost is still being read.
        nativeMemoryListMock.mockImplementation(() => Promise.resolve({ files: [file("only.md")] }));
        launchCostMock.mockImplementation((_c: unknown, req: { agent_id: string }) =>
            req.agent_id === "a2" ? new Promise(() => {}) : Promise.resolve(cost(1200))
        );
        await setAgents([agent("a1", "Manoz"), agent("a2", "AgentY")]);
        await waitFor(() => expect(cardOf("AgentY")).toHaveTextContent("1 file"));
        expect(cardOf("AgentY")).not.toHaveTextContent("5 files");
        expect(cardOf("AgentY")).not.toHaveTextContent("Starts with");
        expect(listCallsFor("a1")).toBe(1);
    });

    test("a removed agent's pending refetch is cancelled and its subscription is dropped", async () => {
        vi.useFakeTimers();
        render(() => <NativeMemoryManager />);
        await vi.waitFor(() => expect(screen.getAllByText("No memories yet")).toHaveLength(2));
        expect(handlerCount(changed("a2"))).toBe(1);

        fire(changed("a2"));
        await setAgentsTimed([agent("a1", "Manoz")]);
        expect(handlerCount(changed("a2"))).toBe(0);
        const before = listCallsFor("a2");
        await vi.advanceTimersByTimeAsync(500);
        expect(listCallsFor("a2")).toBe(before);
    });

    async function setAgentsTimed(list: AgentDefinition[]) {
        listAgentDefinitionsMock.mockResolvedValue(list);
        fire(WpsEvent.AgentsChanged);
        await vi.advanceTimersByTimeAsync(0);
        await vi.waitFor(() => expect(screen.queryByText("AgentY")).toBeNull());
    }
});

describe("NativeMemoryManager — the debounce", () => {
    test("a change refetches after 250 ms, not before", async () => {
        vi.useFakeTimers();
        render(() => <NativeMemoryManager />);
        await vi.waitFor(() => expect(nativeMemoryListMock).toHaveBeenCalledTimes(2));

        fire(changed("a1"));
        await vi.advanceTimersByTimeAsync(249);
        expect(listCallsFor("a1")).toBe(1);
        await vi.advanceTimersByTimeAsync(1);
        expect(listCallsFor("a1")).toBe(2);
    });

    test("closing the pane cancels a refetch that was waiting", async () => {
        vi.useFakeTimers();
        render(() => <NativeMemoryManager />);
        await vi.waitFor(() => expect(nativeMemoryListMock).toHaveBeenCalledTimes(2));

        fire(changed("a1"));
        cleanup();
        await vi.advanceTimersByTimeAsync(500);
        expect(listCallsFor("a1")).toBe(1);
    });
});

describe("NativeMemoryManager — switching agent while a list is in flight", () => {
    /** Calls a1's detail list gets once the grid has settled: the first of a1's next calls. */
    async function openThenSwitch(a1: Promise<unknown>, a2: Promise<unknown>, beforeSwitch?: () => Promise<unknown>) {
        const p = paneWith(null);
        render(() => <NativeMemoryManager pane={p.pane} />);
        await waitFor(() => expect(screen.getAllByText("No memories yet")).toHaveLength(2));
        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) =>
            req.agent_id === "a1" ? a1 : a2
        );
        fireEvent.click(screen.getByText("Manoz"));
        await waitFor(() => expect(openAgentName()).toBe("Manoz"));
        await beforeSwitch?.();
        p.setValue("a2");
        await waitFor(() => expect(openAgentName()).toBe("AgentY"));
        return p;
    }

    test("a late list of the agent left behind is ignored", async () => {
        const a1 = deferred<{ files: NativeMemoryFileMeta[] }>();
        await openThenSwitch(a1.promise, Promise.resolve({ files: [file("two.md")] }));
        await waitFor(() => expect(tileNames()).toEqual(["two.md"]));

        a1.resolve({ files: [file("one.md")] });
        await flush();
        expect(tileNames()).toEqual(["two.md"]);
    });

    test("a late failure of the agent left behind shows no error", async () => {
        const a1 = deferred<{ files: NativeMemoryFileMeta[] }>();
        await openThenSwitch(a1.promise, Promise.resolve({ files: [file("two.md")] }));
        await waitFor(() => expect(tileNames()).toEqual(["two.md"]));

        a1.reject(new Error("late"));
        await flush();
        expect(document.querySelector(".native-memory-manager-error")).toBeNull();
    });

    test("the agent left behind finishing does not end the new agent's loading", async () => {
        const a1 = deferred<{ files: NativeMemoryFileMeta[] }>();
        const a2 = deferred<{ files: NativeMemoryFileMeta[] }>();
        await openThenSwitch(a1.promise, a2.promise);
        expect(screen.getByText("Loading files…")).toBeInTheDocument();

        a1.resolve({ files: [file("one.md")] });
        await flush();
        expect(screen.getByText("Loading files…")).toBeInTheDocument();
        expect(createProps?.listUnknown).toBe(true);

        a2.resolve({ files: [file("two.md")] });
        await waitFor(() => expect(tileNames()).toEqual(["two.md"]));
    });

    test("the new agent starts clean: no files, no 'unverified' mark, no error of the last one", async () => {
        const a2 = deferred<{ files: NativeMemoryFileMeta[] }>();
        await openThenSwitch(Promise.resolve({ files: [file("one.md")], unverified: true }), a2.promise);
        // Still loading a2: nothing of a1 is left.
        expect(screen.getByText("Loading files…")).toBeInTheDocument();
        expect(document.querySelector(".native-memory-manager-unverified")).toBeNull();

        a2.reject(new Error("boom"));
        expect(await screen.findByText("Failed to list memory files: boom")).toBeInTheDocument();
        expect(tileNames()).toEqual([]);
        expect(screen.getByText("This agent's memory directory could not be read.")).toBeInTheDocument();
        expect(document.querySelector(".native-memory-manager-unverified")).toBeNull();
    });

    test("an error of the last agent is gone while the next one loads", async () => {
        const a2 = deferred<{ files: NativeMemoryFileMeta[] }>();
        await openThenSwitch(Promise.reject(new Error("first broke")), a2.promise, async () => {
            expect(await screen.findByText("Failed to list memory files: first broke")).toBeInTheDocument();
        });
        expect(screen.queryByText("Failed to list memory files: first broke")).toBeNull();
        expect(document.querySelector(".native-memory-manager-error")).toBeNull();
        a2.resolve({ files: [] });
        await flush();
    });
});

describe("NativeMemoryManager — an open agent", () => {
    async function openManoz() {
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz"));
        await waitFor(() => expect(openAgentName()).toBe("Manoz"));
    }

    test("opening an agent reads its launch cost again", async () => {
        render(() => <NativeMemoryManager />);
        await waitFor(() => expect(launchCallsFor("a1")).toBe(1));
        fireEvent.click(await screen.findByText("Manoz"));
        await waitFor(() => expect(launchCallsFor("a1")).toBe(2));
    });

    test("the panels above the files are given the agent's id and name", async () => {
        await openManoz();
        expect(await screen.findByTestId("adoption-panel")).toHaveTextContent("a1|Manoz");
        expect(screen.getByTestId("claims-panel")).toHaveTextContent("a1|Manoz");
    });

    test("an agent found by a guess is marked and read-only; a folder it launched with is not", async () => {
        nativeMemoryListMock.mockResolvedValue({ files: [file("MEMORY.md", { is_index: true })], unverified: true });
        await openManoz();
        expect(await screen.findByText("Unverified folder")).toBeInTheDocument();
        await waitFor(() => expect(createProps?.readOnly).toBe(true));
    });

    test("files sort index first, then by tokens, largest first, then by name", async () => {
        nativeMemoryListMock.mockResolvedValue({
            files: [
                file("z.md", { tokens_est: 10 }),
                file("A.md", { tokens_est: 10 }),
                file("big.md", { tokens_est: 500 }),
                file("MEMORY.md", { tokens_est: 1, is_index: true }),
                file("mid.md", { tokens_est: 100 }),
                file("none.md", { tokens_est: undefined }),
            ],
        });
        await openManoz();
        await waitFor(() => expect(tileNames()).toEqual(["MEMORY.md", "big.md", "mid.md", "A.md", "z.md", "none.md"]));
    });

    test.each([
        ["an index file", { files: [file("MEMORY.md", { is_index: true })] }, true, false],
        ["no index file", { files: [file("notes.md")] }, false, false],
    ])("the create row is told, for %s, hasIndex=%s listUnknown=%s", async (_what, list, hasIndex, listUnknown) => {
        nativeMemoryListMock.mockResolvedValue(list);
        await openManoz();
        await waitFor(() => expect(tileNames().length).toBeGreaterThan(0));
        expect(createProps?.hasIndex).toBe(hasIndex);
        expect(createProps?.listUnknown).toBe(listUnknown);
        expect(createProps?.filenames).toEqual(list.files.map((f) => f.filename));
    });

    test("while the list loads, and when it failed, the create row can't tell and is told so", async () => {
        const pending = deferred<{ files: NativeMemoryFileMeta[] }>();
        render(() => <NativeMemoryManager />);
        await waitFor(() => expect(screen.getAllByText("No memories yet")).toHaveLength(2));
        nativeMemoryListMock.mockReturnValue(pending.promise);
        fireEvent.click(screen.getByText("Manoz"));
        await waitFor(() => expect(createProps?.listUnknown).toBe(true));
        expect(createProps?.hasIndex).toBe(true);

        pending.reject(new Error("boom"));
        await screen.findByText("Failed to list memory files: boom");
        expect(createProps?.listUnknown).toBe(true);
        expect(createProps?.hasIndex).toBe(true);
    });

    test("a file created opens, but only from the grid and only for the agent still open", async () => {
        nativeMemoryListMock.mockResolvedValue({ files: [file("one.md")] });
        await openManoz();
        await waitFor(() => expect(tileNames()).toEqual(["one.md"]));
        const calls = listCallsFor("a1");

        // Made for another agent meanwhile: not ours to open, nothing to refresh.
        createProps?.onCreated("elsewhere.md", "a2");
        await flush();
        expect(listCallsFor("a1")).toBe(calls);
        expect(screen.queryByTestId("history-panel")).toBeNull();

        nativeMemoryListMock.mockResolvedValue({ files: [file("one.md"), file("new.md")] });
        createProps?.onCreated("new.md", "a1");
        expect(await screen.findByTestId("history-panel")).toHaveTextContent("a1:new.md");
        expect(listCallsFor("a1")).toBe(calls + 1);
    });

    test("a file created while another is open leaves that one open", async () => {
        nativeMemoryListMock.mockResolvedValue({ files: [file("one.md")] });
        await openManoz();
        fireEvent.click(
            await waitFor(() => {
                const el = document.querySelector<HTMLElement>('.memory-file-card[data-filename="one.md"]');
                if (!el) throw new Error("no tile");
                return el;
            })
        );
        createProps?.onCreated("new.md", "a1");
        await flush();
        expect(screen.getByTestId("history-panel")).toHaveTextContent("a1:one.md");
    });
});

describe("NativeMemoryManager — a live change to the open agent's files", () => {
    async function openWith(list: () => unknown) {
        vi.useFakeTimers();
        nativeMemoryListMock.mockImplementation(() => Promise.resolve(list()));
        render(() => <NativeMemoryManager />);
        await vi.waitFor(() => expect(document.querySelectorAll(".memory-agent-card").length).toBe(2));
        await vi.advanceTimersByTimeAsync(0);
        fireEvent.click(screen.getByText("Manoz"));
        await vi.waitFor(() => expect(openAgentName()).toBe("Manoz"));
    }
    const tick = async () => {
        fire(changed("a1"));
        await vi.advanceTimersByTimeAsync(250);
    };
    const openTile = async (name: string) => {
        fireEvent.click(
            await vi.waitFor(() => {
                const el = document.querySelector<HTMLElement>(`.memory-file-card[data-filename="${name}"]`);
                if (!el) throw new Error("no tile");
                return el;
            })
        );
    };

    test("the open view is reloaded in place each time", async () => {
        await openWith(() => ({ files: [file("one.md")] }));
        await openTile("one.md");
        expect(screen.getByTestId("history-panel")).toHaveAttribute("data-refresh-nonce", "0");
        await tick();
        expect(screen.getByTestId("history-panel")).toHaveAttribute("data-refresh-nonce", "1");
        await tick();
        expect(screen.getByTestId("history-panel")).toHaveAttribute("data-refresh-nonce", "2");
        expect(historyPanelMountCount).toBe(1);
    });

    test("a file that is gone closes its view, unless it holds unsaved edits", async () => {
        let files = [file("one.md"), file("two.md")];
        await openWith(() => ({ files }));
        await openTile("one.md");
        files = [file("two.md")];
        await tick();
        expect(screen.queryByTestId("history-panel")).toBeNull();
        expect(tileNames()).toEqual(["two.md"]);

        files = [file("one.md"), file("two.md")];
        await tick();
        await openTile("one.md");
        reportDirty?.(true);
        files = [file("two.md")];
        await tick();
        expect(screen.getByTestId("history-panel")).toHaveTextContent("a1:one.md");
    });

    test("a file still listed stays open", async () => {
        await openWith(() => ({ files: [file("one.md"), file("two.md")] }));
        await openTile("one.md");
        await tick();
        expect(screen.getByTestId("history-panel")).toHaveTextContent("a1:one.md");
    });

    test("an error is cleared by the next good list, and the 'unverified' mark follows each list", async () => {
        let next: () => Promise<unknown> = () => Promise.reject(new Error("boom"));
        vi.useFakeTimers();
        nativeMemoryListMock.mockImplementation(() => next());
        render(() => <NativeMemoryManager />);
        await vi.waitFor(() => expect(document.querySelectorAll(".memory-agent-card").length).toBe(2));
        await vi.advanceTimersByTimeAsync(0);
        fireEvent.click(screen.getByText("Manoz"));
        await vi.waitFor(() => expect(screen.getByText("Failed to list memory files: boom")).toBeInTheDocument());
        expect(document.querySelector(".native-memory-manager-unverified")).toBeNull();

        next = () => Promise.resolve({ files: [file("one.md")], unverified: true });
        await tick();
        expect(screen.queryByText("Failed to list memory files: boom")).toBeNull();
        expect(screen.getByText("Unverified folder")).toBeInTheDocument();

        next = () => Promise.resolve({ files: [file("one.md")], unverified: false });
        await tick();
        expect(document.querySelector(".native-memory-manager-unverified")).toBeNull();
    });

    test("a failed refresh says so, and a good one after it ends the loading state", async () => {
        let next: () => Promise<unknown> = () => Promise.resolve({ files: [file("one.md")] });
        vi.useFakeTimers();
        nativeMemoryListMock.mockImplementation(() => next());
        render(() => <NativeMemoryManager />);
        await vi.waitFor(() => expect(document.querySelectorAll(".memory-agent-card").length).toBe(2));
        await vi.advanceTimersByTimeAsync(0);
        fireEvent.click(screen.getByText("Manoz"));
        await vi.waitFor(() => expect(tileNames()).toEqual(["one.md"]));

        next = () => Promise.reject(new Error("later"));
        await tick();
        expect(screen.getByText("Failed to list memory files: later")).toBeInTheDocument();
    });
});

describe("NativeMemoryManager — the file grid's empty states", () => {
    test("an empty folder says nothing is remembered; an unreadable one says it could not be read", async () => {
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz"));
        expect(await screen.findByText("This agent hasn't remembered anything yet.")).toBeInTheDocument();
        expect(screen.queryByText("This agent's memory directory could not be read.")).toBeNull();
    });
});
