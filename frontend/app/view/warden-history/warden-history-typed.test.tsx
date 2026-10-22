// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryAgent } from "@/types/rpc/HistoryAgent";
import type { InputHistoryItem } from "@/types/rpc/InputHistoryItem";

const HistoryAgentsCommand = vi.fn();
const AgentInputHistoryCommand = vi.fn();
const AgentInputHistorySearchCommand = vi.fn();
const AgentInputHistoryDeleteCommand = vi.fn();
const AgentInputHistoryClearCommand = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        HistoryAgentsCommand: (...a: unknown[]) => HistoryAgentsCommand(...a),
        AgentInputHistoryCommand: (...a: unknown[]) => AgentInputHistoryCommand(...a),
        AgentInputHistorySearchCommand: (...a: unknown[]) => AgentInputHistorySearchCommand(...a),
        AgentInputHistoryDeleteCommand: (...a: unknown[]) => AgentInputHistoryDeleteCommand(...a),
        AgentInputHistoryClearCommand: (...a: unknown[]) => AgentInputHistoryClearCommand(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { WardenHistoryTyped } from "./warden-history-typed";

const agent = (id: string, extra: Partial<HistoryAgent> = {}): HistoryAgent => ({
    definition_id: id,
    name: id.toUpperCase(),
    provider: "claude",
    fork_of: "",
    fork_of_name: "",
    deleted: false,
    current_bytes: 1,
    archives: 0,
    archive_bytes: 0,
    last_active_ms: 1,
    ...extra,
});

const item = (id: number, text: string, extra: Partial<InputHistoryItem> = {}): InputHistoryItem => ({
    id,
    agent_id: "a1",
    sent_at_ms: Date.UTC(2026, 9, 20, 12, 0, 0) + id * 60_000,
    text,
    kind: "message",
    source: "composer",
    sends: 1,
    ...extra,
});

describe("Warden History → Typed", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        HistoryAgentsCommand.mockResolvedValue({
            agents: [agent("a1"), agent("a2"), agent("gone", { deleted: true })],
        });
        AgentInputHistoryCommand.mockImplementation(async (_c: unknown, d: { agent_id: string }) => ({
            entries: d.agent_id === "a1" ? [item(3, "merge on approval"), item(2, "/compact", { kind: "slash" }), item(1, "hello")] : [],
        }));
        AgentInputHistorySearchCommand.mockResolvedValue({ entries: [item(3, "merge on approval")] });
        AgentInputHistoryDeleteCommand.mockResolvedValue({ removed: 1 });
        AgentInputHistoryClearCommand.mockResolvedValue({ removed: 3 });
    });
    afterEach(() => cleanup());

    it("loads nothing until it is shown", () => {
        render(() => <WardenHistoryTyped visible={false} />);
        expect(HistoryAgentsCommand).not.toHaveBeenCalled();
        expect(AgentInputHistoryCommand).not.toHaveBeenCalled();
    });

    it("shows the first agent's entries newest first, and only agents that still exist", async () => {
        const { container } = render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("merge on approval")).toBeInTheDocument());
        expect(AgentInputHistoryCommand).toHaveBeenCalledWith({}, { agent_id: "a1", before_id: undefined, limit: 200 });
        const rows = Array.from(container.querySelectorAll("tr[data-entry]")).map((r) => r.getAttribute("data-entry"));
        expect(rows).toEqual(["3", "2", "1"]);
        expect(screen.getByText("slash command")).toBeInTheDocument();
        const options = Array.from(container.querySelectorAll("option")).map((o) => o.getAttribute("value"));
        expect(options).toEqual(["a1", "a2"]);
    });

    it("switching agent reads that agent, and says when nothing was typed", async () => {
        const { container } = render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("hello")).toBeInTheDocument());
        fireEvent.change(container.querySelector("select")!, { target: { value: "a2" } });
        await waitFor(() => expect(screen.getByText("Nothing typed to this agent yet.")).toBeInTheDocument());
        expect(AgentInputHistoryCommand).toHaveBeenLastCalledWith({}, { agent_id: "a2", before_id: undefined, limit: 200 });
    });

    it("searches this agent's history", async () => {
        render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("hello")).toBeInTheDocument());
        fireEvent.input(screen.getByPlaceholderText("Search what was typed"), { target: { value: "merge" } });
        await waitFor(() => expect(AgentInputHistorySearchCommand).toHaveBeenCalledWith({}, { agent_id: "a1", query: "merge", limit: 200 }));
        await waitFor(() => expect(screen.queryByText("hello")).toBeNull());
        expect(screen.getByText("merge on approval")).toBeInTheDocument();
    });

    it("forgets one entry only after a second click", async () => {
        render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("hello")).toBeInTheDocument());
        fireEvent.click(screen.getAllByRole("button", { name: "Forget" })[2]);
        expect(AgentInputHistoryDeleteCommand).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Forget this" }));
        await waitFor(() => expect(AgentInputHistoryDeleteCommand).toHaveBeenCalledWith({}, { id: 1 }));
        await waitFor(() => expect(screen.queryByText("hello")).toBeNull());
        expect(screen.getByText("/compact")).toBeInTheDocument();
    });

    it("Cancel backs out of forgetting an entry", async () => {
        render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("hello")).toBeInTheDocument());
        fireEvent.click(screen.getAllByRole("button", { name: "Forget" })[0]);
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(AgentInputHistoryDeleteCommand).not.toHaveBeenCalled();
        expect(screen.getByText("merge on approval")).toBeInTheDocument();
    });

    it("clears an agent's whole history only after naming it and a second click", async () => {
        render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("hello")).toBeInTheDocument());
        fireEvent.click(screen.getByRole("button", { name: "Clear…" }));
        expect(AgentInputHistoryClearCommand).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Forget everything typed to A1" }));
        await waitFor(() => expect(AgentInputHistoryClearCommand).toHaveBeenCalledWith({}, { agent_id: "a1" }));
        await waitFor(() => expect(screen.getByText("Nothing typed to this agent yet.")).toBeInTheDocument());
    });

    it("offers older entries when a full page came back", async () => {
        const page = Array.from({ length: 200 }, (_, i) => item(500 - i, `msg ${500 - i}`));
        AgentInputHistoryCommand.mockResolvedValueOnce({ entries: page }).mockResolvedValueOnce({ entries: [item(300, "msg 300 older")] });
        render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("msg 500")).toBeInTheDocument());
        fireEvent.click(screen.getByRole("button", { name: "Show older" }));
        await waitFor(() => expect(AgentInputHistoryCommand).toHaveBeenLastCalledWith({}, { agent_id: "a1", before_id: 301, limit: 200 }));
        await waitFor(() => expect(screen.getByText("msg 300 older")).toBeInTheDocument());
    });

    it("says why when the history can't be read", async () => {
        AgentInputHistoryCommand.mockRejectedValue(new Error("srv is busy"));
        render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText(/srv is busy/)).toBeInTheDocument());
    });
});
