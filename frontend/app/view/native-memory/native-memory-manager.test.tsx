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
    return Array.from(container.querySelectorAll(".memory-agent-card-title")).map(
        (el) => el.textContent ?? "",
    );
}

const listAgentDefinitionsMock = vi.fn();
const nativeMemoryListMock = vi.fn();

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListAgentDefinitionsCommand: (...args: unknown[]) => listAgentDefinitionsMock(...args),
        NativeMemoryListCommand: (...args: unknown[]) => nativeMemoryListMock(...args),
    },
}));

// The history panel does its own RPC work on mount and is covered by its own
// tests; this suite is about navigation and the grid.
vi.mock("@/app/view/agent/components/NativeMemoryHistoryPanel", () => ({
    NativeMemoryHistoryPanel: (props: { agentId: string; filename: string }) => (
        <div data-testid="history-panel">{`${props.agentId}:${props.filename}`}</div>
    ),
}));

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
                : Promise.resolve({ files: [{ filename: "only.md" }] }),
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
                : Promise.resolve({ files: [] }),
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
            }),
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
                : Promise.resolve({ files: [{ filename: "only.md" }] }),
        );
        render(() => <NativeMemoryManager />);
        expect(await screen.findByText("Couldn't read memories")).toBeInTheDocument();
        expect(await screen.findByText("1 file")).toBeInTheDocument();
    });
});

// docs/specs/SPEC_ARMORY_PERSONAL_MEMORY_FILTER_AND_SORT_2026_09_02.md
describe("NativeMemoryManager — filter and sort", () => {
    test("filters the grid by name, case-insensitively", async () => {
        listAgentDefinitionsMock.mockResolvedValue([
            agent("a1", "Manoz"),
            agent("a2", "AgentY"),
            agent("a3", "Nark"),
        ]);
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
            if (req.agent_id === "a2")
                return Promise.resolve({ files: [{ filename: "x" }, { filename: "y" }] }); // 2 files
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
            (screen.getByTestId("memory-agent-filter-toggle").querySelector("input") as HTMLInputElement).checked,
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

        const select = await screen.findByRole("combobox");
        await waitFor(() => expect(select).not.toBeDisabled());
        fireEvent.change(select, { target: { value: "MEMORY.md" } });

        expect(await screen.findByTestId("history-panel")).toHaveTextContent("a1:MEMORY.md");
    });

    test("surfaces the real error in the detail view when the file list fails", async () => {
        nativeMemoryListMock.mockRejectedValue(new Error("has no working directory"));
        render(() => <NativeMemoryManager />);
        fireEvent.click(await screen.findByText("Manoz"));

        expect(await screen.findByText(/has no working directory/)).toBeInTheDocument();
    });
});
