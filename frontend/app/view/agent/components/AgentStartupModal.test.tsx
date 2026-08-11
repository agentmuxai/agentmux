// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for AgentStartupModal — the "Startup" tab that lets an agent select
 * an existing Bundle as its Session Context "Startup Instructions" source.
 * See docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md §5.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const listMemories = vi.fn();
const getAgentContent = vi.fn();
const setAgentContent = vi.fn();
const openOrFocusPaneByView = vi.fn();

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListMemoriesCommand: (...args: unknown[]) => listMemories(...args),
        GetAgentContentCommand: (...args: unknown[]) => getAgentContent(...args),
        SetAgentContentCommand: (...args: unknown[]) => setAgentContent(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({
    openOrFocusPaneByView: (...args: unknown[]) => openOrFocusPaneByView(...args),
}));

import { AgentStartupModal } from "./AgentStartupModal";

function mkBundle(overrides: Partial<Memory>): Memory {
    return {
        id: "bundle-1",
        name: "Code Reviewer",
        is_blank: false,
        is_global: false,
        instructions: "Review the diff for bugs.",
        ...overrides,
    } as Memory;
}

describe("AgentStartupModal", () => {
    beforeEach(() => {
        listMemories.mockReset();
        getAgentContent.mockReset();
        setAgentContent.mockReset();
        openOrFocusPaneByView.mockReset();
    });

    afterEach(() => {
        cleanup();
    });

    it("lists non-blank bundles and pre-selects the agent's current startup bundle", async () => {
        listMemories.mockResolvedValue([
            mkBundle({ id: "blank", name: "Vanilla CLI", is_blank: true }),
            mkBundle({ id: "bundle-1", name: "Code Reviewer" }),
            mkBundle({ id: "bundle-2", name: "Release Notes Writer" }),
        ]);
        getAgentContent.mockResolvedValue({ content: "bundle-2" });

        render(() => <AgentStartupModal agentId="agent-1" />);

        await waitFor(() => {
            expect(screen.getByRole("combobox")).toBeInTheDocument();
        });

        expect(screen.getByText("Code Reviewer")).toBeInTheDocument();
        expect(screen.getByText("Release Notes Writer")).toBeInTheDocument();
        expect(screen.queryByText("Vanilla CLI")).not.toBeInTheDocument(); // blank singleton filtered out

        const select = screen.getByRole("combobox") as HTMLSelectElement;
        expect(select.value).toBe("bundle-2");
    });

    it("saves immediately when a bundle is selected", async () => {
        listMemories.mockResolvedValue([mkBundle({ id: "bundle-1", name: "Code Reviewer" })]);
        getAgentContent.mockResolvedValue(null);
        setAgentContent.mockResolvedValue({});

        render(() => <AgentStartupModal agentId="agent-1" />);

        const select = await screen.findByRole("combobox");
        fireEvent.change(select, { target: { value: "bundle-1" } });

        await waitFor(() => {
            expect(setAgentContent).toHaveBeenCalledWith(
                {},
                { agent_id: "agent-1", content_type: "startup_bundle_id", content: "bundle-1" },
            );
        });
    });

    it("shows the Armory edit note only once a bundle is selected", async () => {
        listMemories.mockResolvedValue([mkBundle({ id: "bundle-1", name: "Code Reviewer" })]);
        getAgentContent.mockResolvedValue(null);

        render(() => <AgentStartupModal agentId="agent-1" />);
        await screen.findByRole("combobox");
        expect(screen.queryByText(/Armory → ABF/)).not.toBeInTheDocument();

        setAgentContent.mockResolvedValue({});
        const select = screen.getByRole("combobox");
        fireEvent.change(select, { target: { value: "bundle-1" } });

        await waitFor(() => {
            expect(screen.getByText(/Armory → ABF/)).toBeInTheDocument();
        });
    });

    it("clearing the selection back to None saves an empty content_type value", async () => {
        listMemories.mockResolvedValue([mkBundle({ id: "bundle-1", name: "Code Reviewer" })]);
        getAgentContent.mockResolvedValue({ content: "bundle-1" });
        setAgentContent.mockResolvedValue({});

        render(() => <AgentStartupModal agentId="agent-1" />);
        const select = (await screen.findByRole("combobox")) as HTMLSelectElement;
        expect(select.value).toBe("bundle-1");

        fireEvent.change(select, { target: { value: "" } });

        await waitFor(() => {
            expect(setAgentContent).toHaveBeenCalledWith(
                {},
                { agent_id: "agent-1", content_type: "startup_bundle_id", content: "" },
            );
        });
    });
});
