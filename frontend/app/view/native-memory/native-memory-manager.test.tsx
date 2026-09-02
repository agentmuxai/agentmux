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
