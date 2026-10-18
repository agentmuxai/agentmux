// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryAgent } from "@/types/rpc/HistoryAgent";

const HistoryAgentsCommand = vi.fn();
const HistoryAgentSessionsCommand = vi.fn();
const HistoryUsageCommand = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        HistoryAgentsCommand: (...a: unknown[]) => HistoryAgentsCommand(...a),
        HistoryAgentSessionsCommand: (...a: unknown[]) => HistoryAgentSessionsCommand(...a),
        HistoryUsageCommand: (...a: unknown[]) => HistoryUsageCommand(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const openOrFocusHistoryTab = vi.fn();
vi.mock("@/app/view/agent/open-history-tab", () => ({
    openOrFocusHistoryTab: (...a: unknown[]) => openOrFocusHistoryTab(...a),
}));

import { WardenHistoryManager } from "./warden-history-manager";

const agent = (id: string, extra: Partial<HistoryAgent> = {}): HistoryAgent => ({
    definition_id: id,
    name: id,
    provider: "claude",
    fork_of: "",
    fork_of_name: "",
    deleted: false,
    current_bytes: 2048,
    archives: 0,
    archive_bytes: 0,
    last_active_ms: Date.now() - 60_000,
    ...extra,
});

describe("Warden History section", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        HistoryAgentsCommand.mockResolvedValue({
            agents: [
                agent("parent-1", { name: "Parent", archives: 2, archive_bytes: 1024 }),
                agent("fork-1", { name: "Parent 2", fork_of: "parent-1", fork_of_name: "Parent" }),
                agent("gone-123456", { name: "", provider: "", deleted: true }),
            ],
        });
        HistoryAgentSessionsCommand.mockResolvedValue({
            sessions: [{ session_id: "s1234567890", cwd: "/work/a", first_ms: 1, last_ms: 2, bytes: 512, archived: true }],
            problems: [],
        });
        HistoryUsageCommand.mockResolvedValue({
            total_bytes: 4096,
            stores: [{ key: "pane-history", label: "Pane history", bytes: 4096, files: 3 }],
            agents: [],
            computed_at_ms: Date.now(),
        });
    });
    afterEach(() => cleanup());

    it("loads nothing until it is shown", () => {
        render(() => <WardenHistoryManager wardenBlockId="warden-1" visible={false} />);
        expect(HistoryAgentsCommand).not.toHaveBeenCalled();
    });

    it("lists every agent with history, forks under their parent, deleted agents included", async () => {
        const { container } = render(() => <WardenHistoryManager wardenBlockId="warden-1" visible={true} />);
        await waitFor(() => expect(screen.getByText("Parent")).toBeInTheDocument());
        const rows = Array.from(container.querySelectorAll("tr[data-agent]")).map((r) => r.getAttribute("data-agent"));
        expect(rows).toEqual(["parent-1", "fork-1", "gone-123456"]);
        expect(screen.getByText("fork of Parent")).toBeInTheDocument();
        expect(screen.getByText("Deleted agent")).toBeInTheDocument();
        expect(screen.getByText(/3 agents · .* kept · 2 archived conversations/)).toBeInTheDocument();
    });

    it("shows an agent's sessions from its record", async () => {
        render(() => <WardenHistoryManager wardenBlockId="warden-1" visible={true} />);
        await waitFor(() => expect(screen.getByText("Parent")).toBeInTheDocument());
        fireEvent.click(screen.getAllByRole("button", { name: "Show sessions" })[0]);
        await waitFor(() => expect(screen.getByText("s1234567")).toBeInTheDocument());
        expect(HistoryAgentSessionsCommand).toHaveBeenCalledWith({}, { definition_id: "parent-1" });
        expect(screen.getByRole("cell", { name: "Archived" })).toBeInTheDocument();
        expect(screen.getByText("/work/a")).toBeInTheDocument();
    });

    it("opens an agent's history in the Warden pane", async () => {
        render(() => <WardenHistoryManager wardenBlockId="warden-1" visible={true} />);
        await waitFor(() => expect(screen.getByText("Parent")).toBeInTheDocument());
        fireEvent.click(screen.getAllByRole("button", { name: "Open history" })[0]);
        expect(openOrFocusHistoryTab).toHaveBeenCalledWith({
            currentBlockId: "warden-1",
            agentId: "parent-1",
            provider: "claude",
            agentName: "Parent",
        });
    });

    it("Storage shows where history is kept and that it is kept forever", async () => {
        render(() => <WardenHistoryManager wardenBlockId="warden-1" visible={true} />);
        fireEvent.click(screen.getByRole("radio", { name: "Storage" }));
        await waitFor(() => expect(screen.getByText("Pane history")).toBeInTheDocument());
        expect(HistoryUsageCommand).toHaveBeenCalledWith({}, { refresh: false });
        expect(screen.getByText(/Agent history is kept forever/)).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Measure again" }));
        await waitFor(() => expect(HistoryUsageCommand).toHaveBeenCalledWith({}, { refresh: true }));
    });
});
