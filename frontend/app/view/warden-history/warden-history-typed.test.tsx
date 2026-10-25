// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { InputHistoryItem } from "@/types/rpc/InputHistoryItem";

const ListAgentDefinitionsCommand = vi.fn();
const AgentInputHistoryCommand = vi.fn();
const AgentInputHistorySearchCommand = vi.fn();
const AgentInputHistoryTrashCommand = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListAgentDefinitionsCommand: (...a: unknown[]) => ListAgentDefinitionsCommand(...a),
        AgentInputHistoryCommand: (...a: unknown[]) => AgentInputHistoryCommand(...a),
        AgentInputHistorySearchCommand: (...a: unknown[]) => AgentInputHistorySearchCommand(...a),
        AgentInputHistoryTrashCommand: (...a: unknown[]) => AgentInputHistoryTrashCommand(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
// Forgetting and restoring go through the composers' shared cache, which also calls srv.
const forgetInputHistoryEntry = vi.fn();
const clearInputHistory = vi.fn();
const restoreInputHistory = vi.fn();
vi.mock("@/app/view/agent/agent-input-history", () => ({
    forgetInputHistoryEntry: (...a: unknown[]) => forgetInputHistoryEntry(...a),
    clearInputHistory: (...a: unknown[]) => clearInputHistory(...a),
    restoreInputHistory: (...a: unknown[]) => restoreInputHistory(...a),
}));

import { WardenHistoryTyped } from "./warden-history-typed";

const def = (id: string) => ({ id, name: id.toUpperCase() });

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
        ListAgentDefinitionsCommand.mockResolvedValue([def("a2"), def("a1")]);
        AgentInputHistoryCommand.mockImplementation(async (_c: unknown, d: { agent_id: string }) => ({
            entries: d.agent_id === "a1" ? [item(3, "merge on approval"), item(2, "/compact", { kind: "slash" }), item(1, "hello")] : [],
        }));
        AgentInputHistorySearchCommand.mockResolvedValue({ entries: [item(3, "merge on approval")] });
        forgetInputHistoryEntry.mockResolvedValue(undefined);
        clearInputHistory.mockResolvedValue(undefined);
        restoreInputHistory.mockResolvedValue(1);
        AgentInputHistoryTrashCommand.mockResolvedValue({ batches: [], retention_ms: 7 * 24 * 60 * 60 * 1000 });
    });
    afterEach(() => cleanup());

    it("lists what was forgotten with who did it, and Restore brings it back", async () => {
        const batch = { batch_id: "b-1", agent_id: "a1", deleted_at_ms: Date.UTC(2026, 9, 21, 9, 0, 0), deleted_by: "korp", count: 1, sample: "ship the fix" };
        AgentInputHistoryTrashCommand.mockResolvedValueOnce({ batches: [batch], retention_ms: 7 * 24 * 60 * 60 * 1000 });
        const { container } = render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("ship the fix")).toBeInTheDocument());
        expect(AgentInputHistoryTrashCommand).toHaveBeenCalledWith({}, { agent_id: "a1" });
        expect(screen.getByText("by korp")).toBeInTheDocument();
        expect(screen.getByText(/restorable for 7 days/)).toBeInTheDocument();
        AgentInputHistoryTrashCommand.mockResolvedValue({ batches: [], retention_ms: 7 * 24 * 60 * 60 * 1000 });
        fireEvent.click(screen.getByRole("button", { name: "Restore" }));
        await waitFor(() => expect(restoreInputHistory).toHaveBeenCalledWith("b-1"));
        await waitFor(() => expect(container.querySelector("[data-batch]")).toBeNull());
    });

    it("a cleared history shows as one restorable batch", async () => {
        const batch = { batch_id: "b-2", agent_id: "a1", deleted_at_ms: Date.UTC(2026, 9, 21, 9, 0, 0), deleted_by: "armory-ui", count: 3, sample: "hello" };
        AgentInputHistoryTrashCommand.mockResolvedValue({ batches: [batch], retention_ms: 7 * 24 * 60 * 60 * 1000 });
        render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("3 entries, from “hello”")).toBeInTheDocument());
    });

    it("loads nothing until it is shown", () => {
        render(() => <WardenHistoryTyped visible={false} />);
        expect(ListAgentDefinitionsCommand).not.toHaveBeenCalled();
        expect(AgentInputHistoryCommand).not.toHaveBeenCalled();
    });

    it("lists every defined agent by name, hidden ones too, and shows the first one's entries newest first", async () => {
        const { container } = render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("merge on approval")).toBeInTheDocument());
        expect(AgentInputHistoryCommand).toHaveBeenCalledWith({}, { agent_id: "a1", before_id: undefined, limit: 200 });
        const rows = Array.from(container.querySelectorAll("tr[data-entry]")).map((r) => r.getAttribute("data-entry"));
        expect(rows).toEqual(["3", "2", "1"]);
        expect(screen.getByText("slash command")).toBeInTheDocument();
        const options = Array.from(container.querySelectorAll("option")).map((o) => o.getAttribute("value"));
        expect(options).toEqual(["a1", "a2"]);
        expect(ListAgentDefinitionsCommand).toHaveBeenCalledWith({}, { include_hidden: true });
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
        expect(forgetInputHistoryEntry).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Forget this" }));
        // Through the shared cache, so an open composer stops recalling it too.
        await waitFor(() => expect(forgetInputHistoryEntry).toHaveBeenCalledWith("a1", 1));
        await waitFor(() => expect(screen.queryByText("hello")).toBeNull());
        expect(screen.getByText("/compact")).toBeInTheDocument();
    });

    it("Cancel backs out of forgetting an entry", async () => {
        render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("hello")).toBeInTheDocument());
        fireEvent.click(screen.getAllByRole("button", { name: "Forget" })[0]);
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(forgetInputHistoryEntry).not.toHaveBeenCalled();
        expect(screen.getByText("merge on approval")).toBeInTheDocument();
    });

    it("clears an agent's whole history only after naming it and a second click", async () => {
        render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("hello")).toBeInTheDocument());
        fireEvent.click(screen.getByRole("button", { name: "Clear…" }));
        expect(clearInputHistory).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Forget everything typed to A1" }));
        await waitFor(() => expect(clearInputHistory).toHaveBeenCalledWith("a1"));
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

    it("a clear that finishes after switching agents does not empty the agent now shown", async () => {
        AgentInputHistoryCommand.mockImplementation(async (_c: unknown, d: { agent_id: string }) => ({
            entries: [item(d.agent_id === "a1" ? 3 : 9, d.agent_id === "a1" ? "for a1" : "for a2")],
        }));
        let finish!: () => void;
        clearInputHistory.mockReturnValue(new Promise<void>((r) => (finish = r)));
        const { container } = render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText("for a1")).toBeInTheDocument());
        fireEvent.click(screen.getByRole("button", { name: "Clear…" }));
        fireEvent.click(screen.getByRole("button", { name: "Forget everything typed to A1" }));
        fireEvent.change(container.querySelector("select")!, { target: { value: "a2" } });
        await waitFor(() => expect(screen.getByText("for a2")).toBeInTheDocument());
        finish();
        await Promise.resolve();
        await Promise.resolve();
        expect(screen.getByText("for a2")).toBeInTheDocument();
        expect(screen.queryByText("Nothing typed to this agent yet.")).toBeNull();
    });

    it("says why the agents can't be listed instead of staying on Loading", async () => {
        ListAgentDefinitionsCommand.mockRejectedValueOnce(new Error("srv is busy"));
        render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText(/srv is busy/)).toBeInTheDocument());
        expect(screen.getByText("No agent to show.")).toBeInTheDocument();
        expect(screen.queryByText("Loading…")).toBeNull();
        // Refresh tries again.
        fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
        await waitFor(() => expect(screen.getByText("merge on approval")).toBeInTheDocument());
    });

    it("says why when the history can't be read", async () => {
        AgentInputHistoryCommand.mockRejectedValue(new Error("srv is busy"));
        render(() => <WardenHistoryTyped visible={true} />);
        await waitFor(() => expect(screen.getByText(/srv is busy/)).toBeInTheDocument());
    });
});
