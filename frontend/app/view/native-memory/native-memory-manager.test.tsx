// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Armory → Memory → Personal: the agent card grid and its drill-in.
 * docs/specs/SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md.
 *
 * The load-bearing case here is the four-state count: an agent whose
 * `agent:memory:list` REJECTS must render differently from one that returns
 * zero files. `memory_dir_for_agent` fails with a hard HTTP 500 (not an empty
 * list) when the memory dir can't be resolved, which is how every
 * blank-`working_directory` agent failed before #2901 — folding that into
 * "No memories yet" would hide the next occurrence of that bug class.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

function cardTitlesInOrder(container: HTMLElement): string[] {
    return Array.from(container.querySelectorAll(".memory-agent-card-title")).map((el) => el.textContent ?? "");
}

/** Filenames currently rendered as tiles in the file grid, in DOM order.
 *  The tile grid replaced the file `<select>` on 2026-09-04, so this is the
 *  successor to the old `select.querySelectorAll("option")` assertions. */
function fileTileNames(): string[] {
    return Array.from(document.querySelectorAll<HTMLElement>(".memory-file-card")).map(
        (el) => el.getAttribute("data-filename") ?? ""
    );
}

/** Drill into one file's version history by clicking its tile — the
 *  successor to `fireEvent.change(select, ...)`. Waits for the tile to exist
 *  first, since the file grid only paints once agent:memory:list resolves. */
async function openFile(filename: string): Promise<void> {
    const tile = await waitFor(() => {
        const el = document.querySelector<HTMLElement>(`.memory-file-card[data-filename="${filename}"]`);
        if (!el) throw new Error(`no file tile for ${filename}`);
        return el;
    });
    fireEvent.click(tile);
}

const listAgentDefinitionsMock = vi.fn();
const nativeMemoryListMock = vi.fn();

// SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md's own tests drive
// `agent:memory:changed:{id}` events through this hub, same pattern as
// bundle-mcp-model.test.ts's own waveEventSubscribe mock — extended here to
// accept the VARIADIC multi-subscription call NativeMemoryManager makes
// (one subscription per grid agent in a single waveEventSubscribe(...) call,
// not bundle-mcp-model.ts's single-subscription shape).
const wpsHub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((...subs: Array<{ eventType: string; handler: (e: unknown) => void }>) => {
        for (const sub of subs) wpsHub.handlers.set(sub.eventType, sub.handler);
        return () => {
            for (const sub of subs) wpsHub.handlers.delete(sub.eventType);
        };
    }),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListAgentDefinitionsCommand: (...args: unknown[]) => listAgentDefinitionsMock(...args),
        NativeMemoryListCommand: (...args: unknown[]) => nativeMemoryListMock(...args),
    },
}));

// The history panel does its own RPC work on mount and is covered by its own
// tests; this suite is about navigation and the grid. `historyPanelMountCount`
// increments once per MOUNT (a Solid component's own function body runs once
// at creation, not per-render) — used by the Codex P1 regression test below
// to prove the panel actually remounted (and so would re-fetch fresh
// history) rather than just re-rendering with identical text content, which
// a plain toHaveTextContent check can't distinguish.
let historyPanelMountCount = 0;
vi.mock("@/app/view/agent/components/NativeMemoryHistoryPanel", () => ({
    NativeMemoryHistoryPanel: (props: { agentId: string; filename: string }) => {
        historyPanelMountCount++;
        return <div data-testid="history-panel">{`${props.agentId}:${props.filename}`}</div>;
    },
}));

import { fileMetaLabel, formatFileAge, formatFileSize } from "./MemoryFileCard";
import { NativeMemoryManager } from "./native-memory-manager";

function agent(id: string, name: string): AgentDefinition {
    return { id, name, slug: id, provider: "claude" } as AgentDefinition;
}

afterEach(() => {
    cleanup();
});

beforeEach(() => {
    listAgentDefinitionsMock.mockReset();
    nativeMemoryListMock.mockReset();
    listAgentDefinitionsMock.mockResolvedValue([agent("a1", "Manoz"), agent("a2", "AgentY")]);
    nativeMemoryListMock.mockResolvedValue({ files: [] });
    localStorage.clear();
    wpsHub.handlers.clear();
    historyPanelMountCount = 0;
});

describe("NativeMemoryManager — agent grid", () => {
    test("renders one card per agent definition", async () => {
        render(() => <NativeMemoryManager />);
        expect(await screen.findByText("Manoz")).toBeInTheDocument();
        expect(await screen.findByText("AgentY")).toBeInTheDocument();
    });

    test("shows a per-agent file count", async () => {
        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) =>
            req.agent_id === "a1"
                ? Promise.resolve({ files: [{ filename: "MEMORY.md" }, { filename: "b.md" }] })
                : Promise.resolve({ files: [{ filename: "only.md" }] })
        );
        render(() => <NativeMemoryManager />);
        expect(await screen.findByText("2 files")).toBeInTheDocument();
        // Singular, not "1 files".
        expect(await screen.findByText("1 file")).toBeInTheDocument();
    });

    test("an agent with no memories reads as empty, not as an error", async () => {
        render(() => <NativeMemoryManager />);
        const empties = await screen.findAllByText("No memories yet");
        expect(empties).toHaveLength(2);
        expect(screen.queryByText("Couldn't read memories")).toBeNull();
    });

    // The regression guard this whole grid design hinges on.
    test("a rejected count renders as an ERROR, distinctly from zero files", async () => {
        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) =>
            req.agent_id === "a1"
                ? Promise.reject(new Error("memory: agent manoz has no working directory"))
                : Promise.resolve({ files: [] })
        );
        render(() => <NativeMemoryManager />);

        // The failing agent says so...
        expect(await screen.findByText("Couldn't read memories")).toBeInTheDocument();
        // ...and the genuinely-empty one still reads as empty — the two are
        // never collapsed into the same message.
        expect(await screen.findByText("No memories yet")).toBeInTheDocument();
    });

    // Codex P2, PR #2917: useAgentDefinitions returns [] both while the first
    // ListAgentDefinitions is in flight AND when there genuinely are none.
    // Without consuming its `loading` accessor the grid flashed "No agents
    // defined yet" on every mount.
    test("shows a loading state, not a false empty state, before agents resolve", async () => {
        let resolveAgents!: (v: AgentDefinition[]) => void;
        listAgentDefinitionsMock.mockReturnValue(
            new Promise<AgentDefinition[]>((r) => {
                resolveAgents = r;
            })
        );
        render(() => <NativeMemoryManager />);

        expect(await screen.findByText("Loading agents…")).toBeInTheDocument();
        expect(screen.queryByText("No agents defined yet.")).toBeNull();

        resolveAgents([agent("a1", "Manoz")]);
        expect(await screen.findByText("Manoz")).toBeInTheDocument();
    });

    test("still reports a genuinely empty agent list once loading settles", async () => {
        listAgentDefinitionsMock.mockResolvedValue([]);
        render(() => <NativeMemoryManager />);
        expect(await screen.findByText("No agents defined yet.")).toBeInTheDocument();
    });

    test("one failing agent does not blank its siblings", async () => {
        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) =>
            req.agent_id === "a1"
                ? Promise.reject(new Error("boom"))
                : Promise.resolve({ files: [{ filename: "only.md" }] })
        );
        render(() => <NativeMemoryManager />);
        expect(await screen.findByText("Couldn't read memories")).toBeInTheDocument();
        expect(await screen.findByText("1 file")).toBeInTheDocument();
    });
});

// docs/specs/SPEC_ARMORY_PERSONAL_MEMORY_FILTER_AND_SORT_2026_09_02.md
describe("NativeMemoryManager — filter and sort", () => {
    test("filters the grid by name, case-insensitively", async () => {
        listAgentDefinitionsMock.mockResolvedValue([agent("a1", "Manoz"), agent("a2", "AgentY"), agent("a3", "Nark")]);
        render(() => <NativeMemoryManager />);
        await screen.findByText("Manoz");

        fireEvent.input(screen.getByTestId("memory-agent-filter-input"), { target: { value: "MAN" } });

        // "Manoz" contains "MAN" case-insensitively; "AgentY" and "Nark" do not.
        expect(await screen.findByText("Manoz")).toBeInTheDocument();
        expect(screen.queryByText("AgentY")).toBeNull();
        expect(screen.queryByText("Nark")).toBeNull();
    });

    test("clearing the filter (button) restores the full grid", async () => {
        render(() => <NativeMemoryManager />);
        await screen.findByText("Manoz");

        const input = screen.getByTestId("memory-agent-filter-input") as HTMLInputElement;
        fireEvent.input(input, { target: { value: "Manoz" } });
        expect(screen.queryByText("AgentY")).toBeNull();

        fireEvent.click(screen.getByTestId("memory-agent-filter-clear"));
        expect(input.value).toBe("");
        expect(await screen.findByText("AgentY")).toBeInTheDocument();
    });

    test("Escape clears the filter without hiding the bar", async () => {
        render(() => <NativeMemoryManager />);
        await screen.findByText("Manoz");

        const input = screen.getByTestId("memory-agent-filter-input") as HTMLInputElement;
        fireEvent.input(input, { target: { value: "Manoz" } });
        fireEvent.keyDown(input, { key: "Escape" });

        expect(input.value).toBe("");
        expect(await screen.findByText("AgentY")).toBeInTheDocument();
        expect(screen.getByTestId("memory-agent-filter-bar")).toBeInTheDocument();
    });

    test("filtering to zero matches shows a distinct 'no match' message", async () => {
        render(() => <NativeMemoryManager />);
        await screen.findByText("Manoz");

        fireEvent.input(screen.getByTestId("memory-agent-filter-input"), {
            target: { value: "nonexistent" },
        });

        expect(await screen.findByText('No agents match "nonexistent"')).toBeInTheDocument();
        // Distinct from the zero-agents-total empty state.
        expect(screen.queryByText("No agents defined yet.")).toBeNull();
    });

    test("name sort orders the grid alphabetically", async () => {
        listAgentDefinitionsMock.mockResolvedValue([agent("a1", "Zed"), agent("a2", "Alpha")]);
        const { container } = render(() => <NativeMemoryManager />);
        await screen.findByText("Zed");

        expect(cardTitlesInOrder(container)).toEqual(["Alpha", "Zed"]);
    });

    test("provider sort groups by provider, then name within each group", async () => {
        listAgentDefinitionsMock.mockResolvedValue([
            { id: "a1", name: "Zed", slug: "a1", provider: "claude" } as AgentDefinition,
            { id: "a2", name: "Beta", slug: "a2", provider: "claude" } as AgentDefinition,
            { id: "a3", name: "Alpha", slug: "a3", provider: "codex" } as AgentDefinition,
        ]);
        const { container } = render(() => <NativeMemoryManager />);
        await screen.findByText("Zed");

        fireEvent.change(screen.getByTestId("memory-agent-sort-select"), { target: { value: "provider" } });

        expect(cardTitlesInOrder(container)).toEqual(["Beta", "Zed", "Alpha"]);
    });

    test("count sort ranks resolved counts first (descending), then loading, then error — never mixed", async () => {
        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) => {
            if (req.agent_id === "a1") return Promise.resolve({ files: [{ filename: "x" }] }); // 1 file
            if (req.agent_id === "a2") return Promise.resolve({ files: [{ filename: "x" }, { filename: "y" }] }); // 2 files
            if (req.agent_id === "a3") return Promise.reject(new Error("boom")); // error
            return new Promise(() => {}); // a4: never resolves — stays "loading"
        });
        listAgentDefinitionsMock.mockResolvedValue([
            agent("a1", "OneFile"),
            agent("a2", "TwoFiles"),
            agent("a3", "Errored"),
            agent("a4", "Stuck"),
        ]);
        const { container } = render(() => <NativeMemoryManager />);
        await screen.findByText("2 files");
        await screen.findByText("Couldn't read memories");

        fireEvent.change(screen.getByTestId("memory-agent-sort-select"), { target: { value: "count" } });

        expect(cardTitlesInOrder(container)).toEqual(["TwoFiles", "OneFile", "Stuck", "Errored"]);
    });

    test("'Has memories' hides zero-count cards but never loading or error ones", async () => {
        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) => {
            if (req.agent_id === "a1") return Promise.resolve({ files: [{ filename: "x" }] }); // has memories
            if (req.agent_id === "a2") return Promise.resolve({ files: [] }); // zero — should hide
            if (req.agent_id === "a3") return Promise.reject(new Error("boom")); // error — must stay visible
            return new Promise(() => {}); // a4 — stuck loading, must stay visible
        });
        listAgentDefinitionsMock.mockResolvedValue([
            agent("a1", "HasFiles"),
            agent("a2", "Empty"),
            agent("a3", "Errored"),
            agent("a4", "Stuck"),
        ]);
        render(() => <NativeMemoryManager />);
        await screen.findByText("HasFiles");
        await screen.findByText("Couldn't read memories");

        fireEvent.click(screen.getByTestId("memory-agent-filter-toggle").querySelector("input")!);

        expect(screen.getByText("HasFiles")).toBeInTheDocument();
        expect(screen.queryByText("Empty")).toBeNull();
        expect(screen.getByText("Errored")).toBeInTheDocument();
        expect(screen.getByText("Stuck")).toBeInTheDocument();
    });

    test("sort choice persists across a remount; filter text and the toggle do not", async () => {
        const first = render(() => <NativeMemoryManager />);
        await screen.findByText("Manoz");

        fireEvent.change(screen.getByTestId("memory-agent-sort-select"), { target: { value: "provider" } });
        fireEvent.input(screen.getByTestId("memory-agent-filter-input"), { target: { value: "Manoz" } });
        fireEvent.click(screen.getByTestId("memory-agent-filter-toggle").querySelector("input")!);
        first.unmount();

        render(() => <NativeMemoryManager />);
        await screen.findByText("Manoz");

        expect((screen.getByTestId("memory-agent-sort-select") as HTMLSelectElement).value).toBe("provider");
        expect((screen.getByTestId("memory-agent-filter-input") as HTMLInputElement).value).toBe("");
        expect(
            (screen.getByTestId("memory-agent-filter-toggle").querySelector("input") as HTMLInputElement).checked
        ).toBe(false);
    });

    test("the filter bar does not render in the per-agent detail view", async () => {
        nativeMemoryListMock.mockResolvedValue({ files: [{ filename: "MEMORY.md" }] });
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz"));

        expect(await screen.findByText("← All agents")).toBeInTheDocument();
        expect(screen.queryByTestId("memory-agent-filter-bar")).toBeNull();
    });
});

// docs/specs/SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md
describe("NativeMemoryManager — reactive updates", () => {
    test("an agent:memory:changed event refetches only that agent's count", async () => {
        render(() => <NativeMemoryManager />);
        await screen.findAllByText("No memories yet"); // initial fetch settled for both

        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) =>
            req.agent_id === "a1"
                ? Promise.resolve({ files: [{ filename: "MEMORY.md" }] })
                : Promise.resolve({ files: [] })
        );
        wpsHub.handlers.get("agent:memory:changed:a1")?.({});

        expect(await screen.findByText("1 file")).toBeInTheDocument();
        // a2's card never re-fetched (still shows the original empty state).
        expect(screen.getByText("AgentY").closest(".memory-agent-card")).toHaveTextContent("No memories yet");
    });

    // ReAgent (PR #2932, non-blocking follow-up): fetchCountFor previously
    // overwrote to {kind: "loading"} unconditionally, including on a
    // reactive refresh of an ALREADY-resolved card -- a visible
    // "Loading…" flash on every write, inconsistent with the feature's own
    // "quiet in-place update" design intent and with
    // refetchSelectedAgentFiles's own deliberate choice not to touch
    // filesLoading for the identical reason.
    test("a reactive count refresh does not flash 'Loading…' for an already-resolved card", async () => {
        vi.useFakeTimers();
        try {
            nativeMemoryListMock.mockResolvedValue({ files: [{ filename: "MEMORY.md" }] });
            render(() => <NativeMemoryManager />);
            await vi.waitFor(() => expect(screen.getAllByText("1 file")).toHaveLength(2));

            // Only `files.length` is ever read by the code under test here
            // (fetchCountFor), so a minimal shape is honest about what this
            // test actually needs — not the full NativeMemoryFileMeta.
            let resolveRefresh: ((v: { files: { filename: string }[] }) => void) | undefined;
            nativeMemoryListMock.mockImplementation(
                () =>
                    new Promise((resolve) => {
                        resolveRefresh = resolve;
                    })
            );
            wpsHub.handlers.get("agent:memory:changed:a1")?.({});
            // Past the 250ms debounce -- fetchCountFor has now actually been
            // called and its RPC is in flight (held pending by resolveRefresh).
            await vi.advanceTimersByTimeAsync(250);

            // While the reactive refetch is still in flight, a1's card must
            // still show its OLD resolved count, never "Loading…".
            expect(screen.getByText("Manoz").closest(".memory-agent-card")).toHaveTextContent("1 file");
            expect(screen.getByText("Manoz").closest(".memory-agent-card")).not.toHaveTextContent("Loading");

            resolveRefresh?.({ files: [{ filename: "MEMORY.md" }, { filename: "NOTES.md" }] });
            await vi.waitFor(() =>
                expect(screen.getByText("Manoz").closest(".memory-agent-card")).toHaveTextContent("2 files")
            );
        } finally {
            vi.useRealTimers();
        }
    });

    test("an event refetches an agent even if its card previously errored", async () => {
        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) =>
            req.agent_id === "a1" ? Promise.reject(new Error("boom")) : Promise.resolve({ files: [] })
        );
        render(() => <NativeMemoryManager />);
        await screen.findByText("Couldn't read memories");

        // Whatever broke is fixed now — the next event must get a fresh try,
        // not stay permanently errored until a full remount.
        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) =>
            req.agent_id === "a1"
                ? Promise.resolve({ files: [{ filename: "MEMORY.md" }] })
                : Promise.resolve({ files: [] })
        );
        wpsHub.handlers.get("agent:memory:changed:a1")?.({});

        expect(await screen.findByText("1 file")).toBeInTheDocument();
        expect(screen.queryByText("Couldn't read memories")).toBeNull();
    });

    test("rapid repeated events for the same agent debounce into a single refetch", async () => {
        vi.useFakeTimers();
        try {
            render(() => <NativeMemoryManager />);
            await vi.waitFor(() => expect(nativeMemoryListMock).toHaveBeenCalledTimes(2));
            nativeMemoryListMock.mockClear();

            const handler = wpsHub.handlers.get("agent:memory:changed:a1");
            handler?.({});
            handler?.({});
            handler?.({});
            await vi.advanceTimersByTimeAsync(250);

            expect(nativeMemoryListMock).toHaveBeenCalledTimes(1);
        } finally {
            vi.useRealTimers();
        }
    });

    test("an event for the open detail view's agent refreshes its file list in place", async () => {
        nativeMemoryListMock.mockResolvedValue({ files: [{ filename: "MEMORY.md" }] });
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz"));
        await openFile("MEMORY.md");
        expect(await screen.findByTestId("history-panel")).toHaveTextContent("a1:MEMORY.md");

        nativeMemoryListMock.mockResolvedValue({ files: [{ filename: "MEMORY.md" }, { filename: "NOTES.md" }] });
        wpsHub.handlers.get("agent:memory:changed:a1")?.({});

        // Still showing the same file's history — the event refreshed the
        // list in place, it did not kick the user back to the file grid.
        expect(screen.getByTestId("history-panel")).toHaveTextContent("a1:MEMORY.md");
        // ...and stepping back one level shows the REFRESHED list, proving the
        // in-place refresh actually landed rather than being merely harmless.
        fireEvent.click(screen.getByText("← All files"));
        await waitFor(() => expect(fileTileNames()).toContain("NOTES.md"));
    });

    test("an event for a DIFFERENT agent than the one open in detail view is ignored there", async () => {
        nativeMemoryListMock.mockResolvedValue({ files: [{ filename: "MEMORY.md" }] });
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz")); // opens agent a1
        await waitFor(() => expect(fileTileNames()).toEqual(["MEMORY.md"]));

        nativeMemoryListMock.mockClear();
        wpsHub.handlers.get("agent:memory:changed:a2")?.({});

        // a2's own grid card legitimately refetches in the background (the
        // grid stays reactive even while the detail view is open) — what
        // must NOT happen is a refetch for a1, the agent the open detail
        // view is actually showing.
        await vi.waitFor(() => expect(nativeMemoryListMock).toHaveBeenCalledWith(undefined, { agent_id: "a2" }));
        expect(nativeMemoryListMock).not.toHaveBeenCalledWith(undefined, { agent_id: "a1" });
    });

    test("clears the selected filename if the event's refresh shows it no longer exists", async () => {
        nativeMemoryListMock.mockResolvedValue({ files: [{ filename: "MEMORY.md" }] });
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz"));
        await openFile("MEMORY.md");
        expect(await screen.findByTestId("history-panel")).toHaveTextContent("a1:MEMORY.md");

        // The file that was selected is gone in the refreshed list.
        nativeMemoryListMock.mockResolvedValue({ files: [{ filename: "OTHER.md" }] });
        wpsHub.handlers.get("agent:memory:changed:a1")?.({});

        // Falls back to the file grid rather than holding a history panel open
        // on a file that no longer exists.
        await waitFor(() => expect(screen.queryByTestId("history-panel")).toBeNull());
        await waitFor(() => expect(fileTileNames()).toEqual(["OTHER.md"]));
    });

    test("subscriptions are re-registered when the agent set changes", async () => {
        render(() => <NativeMemoryManager />);
        await screen.findByText("Manoz");
        expect(wpsHub.handlers.has("agent:memory:changed:a1")).toBe(true);
        expect(wpsHub.handlers.has("agent:memory:changed:a2")).toBe(true);

        listAgentDefinitionsMock.mockResolvedValue([agent("a3", "Nark")]);
        // Simulate the agents:changed-driven refetch useAgentDefinitions does
        // internally by re-invoking its own subscribe path is out of scope
        // here (that's useAgentDefinitions' own test surface) — instead
        // re-render to exercise the same agentIdsKey-change code path this
        // effect depends on.
        cleanup();
        render(() => <NativeMemoryManager />);
        await screen.findByText("Nark");

        expect(wpsHub.handlers.has("agent:memory:changed:a1")).toBe(false);
        expect(wpsHub.handlers.has("agent:memory:changed:a2")).toBe(false);
        expect(wpsHub.handlers.has("agent:memory:changed:a3")).toBe(true);
    });

    // Codex P1, PR #2932: a live write to the file already open in the
    // detail view is the scenario this whole feature exists for. The panel
    // is keyed on agentId:filename, which don't change when only the SAME
    // file's content changes — without a forced remount, the new content/
    // history stayed invisible until the user switched away and back.
    test("an event for the open file re-mounts the history panel, not just re-renders it", async () => {
        nativeMemoryListMock.mockResolvedValue({ files: [{ filename: "MEMORY.md" }] });
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz"));
        await openFile("MEMORY.md");
        await screen.findByTestId("history-panel");
        const mountsBefore = historyPanelMountCount;

        // Same file list, same selected file — nothing about agentId or
        // filename changes, only the file's own content on the backend.
        wpsHub.handlers.get("agent:memory:changed:a1")?.({});

        await waitFor(() => expect(historyPanelMountCount).toBeGreaterThan(mountsBefore));
        // Still showing the same agent/file — a remount, not a navigation.
        expect(screen.getByTestId("history-panel")).toHaveTextContent("a1:MEMORY.md");
    });

    // Codex P2, PR #2932: refetchSelectedAgentFiles shares latestRequestId
    // with the selectedAgent()-keyed effect. If a change event's refetch
    // completes and becomes the new latestRequestId before the ORIGINAL
    // agent-switch fetch resolves, that original fetch's own .finally()
    // skips setFilesLoading(false) (guarded by the same requestId check) --
    // and if refetchSelectedAgentFiles itself never touched filesLoading,
    // nothing else would ever clear it, leaving the selector permanently
    // disabled/stuck on "Loading…".
    test("does not get stuck loading when an event's refetch resolves before the initial agent-switch fetch", async () => {
        let resolveInitial: ((v: { files: NativeMemoryFileMeta[] }) => void) | undefined;
        nativeMemoryListMock.mockImplementation(
            () =>
                new Promise((resolve) => {
                    resolveInitial = resolve;
                })
        );
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz")); // fires the initial (slow) fetch

        // Still "Loading files…" — the initial fetch has not resolved yet.
        expect(await screen.findByText("Loading files…")).toBeInTheDocument();

        // A change event's refetch fires and resolves BEFORE the initial
        // fetch does — it becomes the new latestRequestId.
        nativeMemoryListMock.mockResolvedValue({ files: [{ filename: "MEMORY.md" }] });
        wpsHub.handlers.get("agent:memory:changed:a1")?.({});
        await waitFor(() => expect(fileTileNames()).toEqual(["MEMORY.md"]));

        // The original (now-superseded) initial fetch finally resolves.
        resolveInitial?.({ files: [] });
        await Promise.resolve();

        // Must still reflect the newer, correct result -- not stuck on
        // "Loading files…", and not clobbered back to the stale initial
        // (empty) result either.
        expect(screen.queryByText("Loading files…")).toBeNull();
        expect(fileTileNames()).toEqual(["MEMORY.md"]);
    });

    // Codex P2, PR #2932: fetchCountFor previously guarded only "is this
    // agent still present", not "is this the newest request for this
    // agent". Two overlapping calls for the same agent (e.g. an unusually
    // slow initial fetch, superseded by a fast event-triggered refetch)
    // could let the OLDER response win if it resolves last.
    test("an older, slower count response cannot overwrite a newer one for the same agent", async () => {
        let resolveSlow: ((v: { files: NativeMemoryFileMeta[] }) => void) | undefined;
        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) => {
            if (req.agent_id !== "a1") return Promise.resolve({ files: [] });
            return new Promise((resolve) => {
                resolveSlow = resolve;
            });
        });
        render(() => <NativeMemoryManager />);
        await screen.findByText("Manoz"); // initial fetch for a1 now in flight (slow)

        // A change event fires a second, faster request for the same agent,
        // which resolves first with the "true" current count.
        nativeMemoryListMock.mockImplementation((_c: unknown, req: { agent_id: string }) =>
            req.agent_id === "a1"
                ? Promise.resolve({ files: [{ filename: "MEMORY.md" }, { filename: "NOTES.md" }] })
                : Promise.resolve({ files: [] })
        );
        wpsHub.handlers.get("agent:memory:changed:a1")?.({});
        await screen.findByText("2 files");

        // The original slow request finally resolves with stale data.
        resolveSlow?.({ files: [] });
        await Promise.resolve();

        // Must still show the newer, correct count on a1's own card
        // specifically — a2's own (genuinely empty) card still legitimately
        // reads "No memories yet" and is not what this assertion is about.
        expect(screen.getByText("2 files")).toBeInTheDocument();
        expect(screen.getByText("Manoz").closest(".memory-agent-card")).toHaveTextContent("2 files");
    });

    // ReAgent P1, PR #2932: an unrelated agent appearing/disappearing
    // ANYWHERE in the app changes agentIdsKey() (it's derived from every
    // agent definition, not just ones with memory) and re-runs this whole
    // subscription effect. A pending debounced refetch for an agent that's
    // still present must survive that re-run, not get silently canceled.
    test("a pending debounced refetch survives an unrelated agent appearing elsewhere within the debounce window", async () => {
        vi.useFakeTimers();
        try {
            render(() => <NativeMemoryManager />);
            await vi.waitFor(() => expect(nativeMemoryListMock).toHaveBeenCalledTimes(2));
            nativeMemoryListMock.mockClear();
            nativeMemoryListMock.mockResolvedValue({ files: [{ filename: "MEMORY.md" }] });

            // Schedule a1's debounced refetch (250ms), then — BEFORE it
            // fires — an unrelated agent (a3) appears elsewhere in the app.
            // useAgentDefinitions() itself subscribes to "agents:changed"
            // (AgentPicker.tsx) via the same mocked waveEventSubscribe hub,
            // so triggering that here re-fetches the agent list for real,
            // exactly as it would from a genuine unrelated create/edit.
            wpsHub.handlers.get("agent:memory:changed:a1")?.({});
            listAgentDefinitionsMock.mockResolvedValue([
                agent("a1", "Manoz"),
                agent("a2", "AgentY"),
                agent("a3", "Nark"),
            ]);
            wpsHub.handlers.get("agents:changed")?.({});
            await vi.waitFor(() => expect(screen.queryByText("Nark")).not.toBeNull());

            // a1's debounce timer must still fire, not have been silently
            // canceled by the resubscribe the agent-list change triggered.
            await vi.advanceTimersByTimeAsync(250);

            expect(nativeMemoryListMock).toHaveBeenCalledWith(undefined, { agent_id: "a1" });
        } finally {
            vi.useRealTimers();
        }
    });
});

describe("NativeMemoryManager — drill-in", () => {
    test("clicking a card opens that agent's detail view, and back returns to the grid", async () => {
        nativeMemoryListMock.mockResolvedValue({ files: [{ filename: "MEMORY.md" }] });
        render(() => <NativeMemoryManager />);

        fireEvent.click(await screen.findByText("Manoz"));

        // Detail view: agent name in the header, back affordance present,
        // grid gone.
        expect(await screen.findByText("← All agents")).toBeInTheDocument();
        expect(screen.queryByText("AgentY")).toBeNull();

        fireEvent.click(screen.getByText("← All agents"));
        expect(await screen.findByText("AgentY")).toBeInTheDocument();
    });

    test("selecting a file mounts the history panel for that agent+file", async () => {
        nativeMemoryListMock.mockResolvedValue({ files: [{ filename: "MEMORY.md" }] });
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz"));
        await openFile("MEMORY.md");

        expect(await screen.findByTestId("history-panel")).toHaveTextContent("a1:MEMORY.md");
    });

    test("surfaces the real error in the detail view when the file list fails", async () => {
        nativeMemoryListMock.mockRejectedValue(new Error("has no working directory"));
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz"));

        expect(await screen.findByText(/has no working directory/)).toBeInTheDocument();
    });
});

// The file grid replaced the detail header's file `<select>` on 2026-09-04 —
// second screen of the drill-down (agents → files → version history). The
// dropdown could only ever show a filename; these cover what the tiles add on
// top of that, and the extra navigation level they introduce.
describe("NativeMemoryManager — file grid", () => {
    /** A full NativeMemoryFileMeta. The older suites above pass bare
     *  `{ filename }` on purpose (they only assert on navigation), but the
     *  tiles render every field, so anything asserting on tile CONTENT needs
     *  the real shape. */
    function fileMeta(filename: string, over: Partial<NativeMemoryFileMeta> = {}): NativeMemoryFileMeta {
        return {
            filename,
            is_index: false,
            metadata_type: null,
            size_bytes: 512,
            modified_at: Date.now() - 2 * 86_400_000,
            ...over,
        };
    }

    async function openAgentWithFiles(files: NativeMemoryFileMeta[]): Promise<void> {
        nativeMemoryListMock.mockResolvedValue({ files });
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz"));
        await waitFor(() => expect(document.querySelectorAll(".memory-file-card").length).toBe(files.length));
    }

    test("renders one tile per memory file instead of a dropdown", async () => {
        await openAgentWithFiles([fileMeta("MEMORY.md"), fileMeta("notes.md")]);

        expect(fileTileNames()).toEqual(["MEMORY.md", "notes.md"]);
        // The <select> is gone, not merely hidden behind the tiles.
        expect(screen.queryByRole("combobox")).toBeNull();
    });

    test("a tile carries the metadata the dropdown could not show", async () => {
        await openAgentWithFiles([fileMeta("feedback_x.md", { metadata_type: "feedback", size_bytes: 2048 })]);

        const tile = document.querySelector<HTMLElement>(".memory-file-card")!;
        expect(tile.querySelector(".memory-file-card-badge")).toHaveTextContent("feedback");
        expect(tile.querySelector(".memory-file-card-meta")).toHaveTextContent("2.0 KB · 2d ago");
    });

    test("the index file is marked and sorts first, whatever its name", async () => {
        // Returned LAST and named to sort last alphabetically — so passing
        // this can only be the explicit is_index rule, not incidental order.
        await openAgentWithFiles([fileMeta("aaa.md"), fileMeta("zzz_index.md", { is_index: true })]);

        expect(fileTileNames()).toEqual(["zzz_index.md", "aaa.md"]);
        const first = document.querySelector<HTMLElement>(".memory-file-card")!;
        expect(first).toHaveClass("memory-file-card--index");
        expect(first.querySelector(".memory-file-card-badge--index")).toHaveTextContent("index");
    });

    test("keyboard activation opens a file, same as a click", async () => {
        await openAgentWithFiles([fileMeta("MEMORY.md")]);

        const tile = document.querySelector<HTMLElement>(".memory-file-card")!;
        expect(tile).toHaveAttribute("role", "button");
        fireEvent.keyDown(tile, { key: "Enter" });

        expect(await screen.findByTestId("history-panel")).toHaveTextContent("a1:MEMORY.md");
    });

    test("back steps one level at a time: history → file grid → agent grid", async () => {
        await openAgentWithFiles([fileMeta("MEMORY.md")]);
        await openFile("MEMORY.md");
        await screen.findByTestId("history-panel");

        // First back returns to the file grid, NOT all the way to the agents.
        fireEvent.click(screen.getByText("← All files"));
        await waitFor(() => expect(fileTileNames()).toEqual(["MEMORY.md"]));
        expect(screen.queryByTestId("history-panel")).toBeNull();
        expect(screen.queryByText("AgentY")).toBeNull(); // still inside the agent

        // Second back returns to the agent grid.
        fireEvent.click(screen.getByText("← All agents"));
        expect(await screen.findByText("AgentY")).toBeInTheDocument();
    });

    test("the open filename shows in the header only on the history screen", async () => {
        await openAgentWithFiles([fileMeta("MEMORY.md")]);
        expect(document.querySelector(".native-memory-manager-filename")).toBeNull();

        await openFile("MEMORY.md");
        await screen.findByTestId("history-panel");
        expect(document.querySelector(".native-memory-manager-filename")).toHaveTextContent("MEMORY.md");
    });

    test("an agent with no files reads as empty, distinctly from a failed read", async () => {
        nativeMemoryListMock.mockResolvedValue({ files: [] });
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz"));

        expect(await screen.findByText("This agent hasn't remembered anything yet.")).toBeInTheDocument();
        expect(document.querySelectorAll(".memory-file-card").length).toBe(0);
    });
});

// Pure helpers behind the tile's meta line. The grid test above covers the
// ordinary case; these are the inputs a real memory dir actually produces
// that have no sensible rendering (a brand-new empty file, a stat that came
// back without a timestamp) and would otherwise surface as "0 B · 20790d ago".
describe("MemoryFileCard — label helpers", () => {
    test("formatFileSize scales through B/KB/MB", () => {
        expect(formatFileSize(0)).toBe("0 B");
        expect(formatFileSize(512)).toBe("512 B");
        expect(formatFileSize(2048)).toBe("2.0 KB");
        expect(formatFileSize(5 * 1024 * 1024)).toBe("5.0 MB");
    });

    test("formatFileAge buckets by magnitude and never renders epoch zero", () => {
        expect(formatFileAge(Date.now() - 30_000)).toBe("just now");
        expect(formatFileAge(Date.now() - 5 * 60_000)).toBe("5m ago");
        expect(formatFileAge(Date.now() - 3 * 3_600_000)).toBe("3h ago");
        expect(formatFileAge(Date.now() - 4 * 86_400_000)).toBe("4d ago");
        // A missing timestamp is dropped, not rendered as 1970.
        expect(formatFileAge(0)).toBe("");
    });

    test("fileMetaLabel omits segments it has no value for, rather than leaving a dangling separator", () => {
        const base = { filename: "x.md", is_index: false, metadata_type: null };
        expect(fileMetaLabel({ ...base, size_bytes: 100, modified_at: 0 })).toBe("100 B");
        expect(fileMetaLabel({ ...base, size_bytes: 0, modified_at: 0 })).toBe("0 B");
    });
});
