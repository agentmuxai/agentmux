// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { list, search, del, clear } = vi.hoisted(() => ({ list: vi.fn(), search: vi.fn(), del: vi.fn(), clear: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentInputHistoryCommand: list,
        AgentInputHistorySearchCommand: search,
        AgentInputHistoryDeleteCommand: del,
        AgentInputHistoryClearCommand: clear,
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

import { movedCursor } from "../agent-input-history";
import { InputHistorySearch } from "./InputHistorySearch";

const entry = (id: number, text: string, sends = 1) => ({ id, agent_id: "a1", sent_at_ms: 1_000, text, kind: "message", source: "composer", sends });

describe("the composer's history cursor", () => {
    it("stays on the same entry as entries arrive or go around it", () => {
        // 3 shown, the cursor on the live draft (3).
        expect(movedCursor(3, 5, "older", 2, 0)).toBe(5);
        expect(movedCursor(3, 4, "newer", 1, 0), "live stays live").toBe(4);
        expect(movedCursor(1, 4, "newer", 1, 0), "an older entry stays put").toBe(1);
        expect(movedCursor(3, 2, "removed", 1, 0), "one before it went").toBe(2);
        expect(movedCursor(0, 2, "removed", 1, 2), "one after it went").toBe(0);
        expect(movedCursor(1, 2, "removed", 1, 1), "its own entry went: the next one").toBe(1);
        expect(movedCursor(3, 0, "removed", 3, 0), "all cleared: live").toBe(0);
    });
});

describe("Ctrl+R history search", () => {
    beforeEach(() => {
        list.mockReset().mockResolvedValue({ entries: [entry(3, "run the tests", 4), entry(2, "deploy to staging"), entry(1, "fix the build")] });
        search.mockReset().mockResolvedValue({ entries: [entry(2, "deploy to staging")] });
        del.mockReset().mockResolvedValue({ removed: 1 });
        clear.mockReset().mockResolvedValue({ removed: 3 });
    });
    afterEach(cleanup);

    it("lists the newest first, filters as you type, and Enter puts the entry in the composer", async () => {
        const done = vi.fn();
        render(() => <InputHistorySearch agentId="a1" onDone={done} />);
        expect(await screen.findByText("run the tests")).toBeTruthy();
        expect(screen.getByText(/4× ·/)).toBeTruthy();
        const box = screen.getByPlaceholderText("Type to filter");
        fireEvent.input(box, { target: { value: "deploy" } });
        await waitFor(() => expect(search).toHaveBeenCalledWith({}, { agent_id: "a1", query: "deploy", limit: 50 }));
        await waitFor(() => expect(screen.queryByText("run the tests")).toBeNull());
        fireEvent.keyDown(box, { key: "Enter" });
        expect(done).toHaveBeenCalledWith("deploy to staging");
    });

    it("moves with the arrows and Ctrl+R, and Escape goes back with nothing", async () => {
        const done = vi.fn();
        render(() => <InputHistorySearch agentId="a1" onDone={done} />);
        await screen.findByText("run the tests");
        const box = screen.getByPlaceholderText("Type to filter");
        fireEvent.keyDown(box, { key: "ArrowDown" });
        fireEvent.keyDown(box, { key: "r", ctrlKey: true });
        fireEvent.keyDown(box, { key: "Enter" });
        expect(done).toHaveBeenLastCalledWith("fix the build");
        fireEvent.keyDown(box, { key: "Escape" });
        expect(done).toHaveBeenLastCalledWith();
    });

    it("Shift+Delete forgets the selected entry; Clear history asks, then forgets them all", async () => {
        render(() => <InputHistorySearch agentId="a1" onDone={() => {}} />);
        await screen.findByText("run the tests");
        fireEvent.keyDown(screen.getByPlaceholderText("Type to filter"), { key: "Delete", shiftKey: true });
        await waitFor(() => expect(screen.queryByText("run the tests")).toBeNull());
        expect(del).toHaveBeenCalledWith({}, { id: 3 });
        fireEvent.click(screen.getByRole("button", { name: "Clear history" }));
        expect(clear).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Forget everything typed to this agent" }));
        await waitFor(() => expect(screen.getByText("Nothing typed to this agent yet.")).toBeTruthy());
        expect(clear).toHaveBeenCalledWith({}, { agent_id: "a1" });
    });

    it("says so when forgetting fails, and keeps the entry", async () => {
        del.mockRejectedValue(new Error("srv went away"));
        render(() => <InputHistorySearch agentId="a1" onDone={() => {}} />);
        await screen.findByText("run the tests");
        fireEvent.keyDown(screen.getByPlaceholderText("Type to filter"), { key: "Delete", shiftKey: true });
        expect(await screen.findByText(/Couldn't forget it: srv went away/)).toBeTruthy();
        expect(screen.getByText("run the tests")).toBeTruthy();
    });
});
