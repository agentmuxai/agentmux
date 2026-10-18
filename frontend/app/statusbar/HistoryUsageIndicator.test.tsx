// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The history total in the status bar and its panel: the total reads in bytes
 * once measured, a click opens the breakdown, and the panel's refresh asks the
 * server to measure again.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@floating-ui/dom", () => ({ autoUpdate: vi.fn(() => vi.fn()) }));
vi.mock("@/app/platform/pane-overlay", () => ({ usePaneOverlay: vi.fn() }));
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({ style: { position: "fixed", left: "0px", top: "0px" } })),
}));

const historyUsageCommand = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { HistoryUsageCommand: (...args: unknown[]) => historyUsageCommand(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { HistoryUsageIndicator } from "./HistoryUsageIndicator";
import { measuredAgo } from "./HistoryUsagePopover";

const GB = 1024 ** 3;
const usage = {
    total_bytes: 3 * GB,
    stores: [
        { key: "provider-files", label: "Session files (the CLI's own)", bytes: 2 * GB, files: 100 },
        { key: "pane-history", label: "Pane history (shared)", bytes: 1 * GB, files: 1 },
    ],
    agents: [{ definition_id: "d1", name: "Lark", bytes: 512 * 1024 * 1024 }],
    computed_at_ms: Date.now() - 3 * 60_000,
};

describe("measuredAgo", () => {
    it("says when the numbers were measured", () => {
        expect(measuredAgo(1_000_000, 1_000_000 + 10_000)).toBe("just now");
        expect(measuredAgo(1_000_000, 1_000_000 + 3 * 60_000)).toBe("3 min ago");
        expect(measuredAgo(1_000_000, 1_000_000 + 125 * 60_000)).toBe("2 h ago");
        expect(measuredAgo(5_000, 1_000), "a clock a little behind is not negative").toBe("just now");
    });
});

describe("HistoryUsageIndicator", () => {
    beforeEach(() => {
        historyUsageCommand.mockReset();
        historyUsageCommand.mockResolvedValue(usage);
    });
    afterEach(() => cleanup());

    it("shows the total once it is measured", async () => {
        render(() => <HistoryUsageIndicator />);
        expect(screen.getByRole("button")).toHaveTextContent("…");
        await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("3.0 GB"));
        expect(historyUsageCommand).toHaveBeenCalledWith(expect.anything(), { refresh: false });
    });

    it("opens the breakdown on a click: each store, the largest agents, and no way to delete", async () => {
        render(() => <HistoryUsageIndicator />);
        await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("3.0 GB"));
        fireEvent.click(screen.getByRole("button", { name: /History kept on this machine/ }));

        const dialog = await screen.findByRole("dialog");
        expect(dialog).toHaveTextContent("kept forever");
        expect(dialog.querySelector('[data-store="provider-files"]')).toHaveTextContent("2.0 GB");
        expect(dialog.querySelector('[data-store="pane-history"]')).toHaveTextContent("1.0 GB");
        expect(dialog.querySelector('[data-agent="d1"]')).toHaveTextContent("Lark");
        expect(dialog).toHaveTextContent("Measured 3 min ago");
        expect(screen.queryByText(/delete|clear|remove/i)).toBeNull();
    });

    it("asks the server to measure again from the panel", async () => {
        render(() => <HistoryUsageIndicator />);
        await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("3.0 GB"));
        fireEvent.click(screen.getByRole("button", { name: /History kept on this machine/ }));
        historyUsageCommand.mockResolvedValue({ ...usage, total_bytes: 4 * GB });

        fireEvent.click(await screen.findByRole("button", { name: "Measure again" }));

        await waitFor(() => expect(historyUsageCommand).toHaveBeenLastCalledWith(expect.anything(), { refresh: true }));
        await waitFor(() => expect(screen.getAllByText("4.0 GB").length).toBeGreaterThan(0));
    });

    it("keeps the last total when a measurement fails", async () => {
        render(() => <HistoryUsageIndicator />);
        await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("3.0 GB"));
        historyUsageCommand.mockRejectedValue(new Error("busy"));
        fireEvent.click(screen.getByRole("button", { name: /History kept on this machine/ }));
        fireEvent.click(await screen.findByRole("button", { name: "Measure again" }));
        await waitFor(() => expect(historyUsageCommand).toHaveBeenCalledTimes(2));
        expect(screen.getAllByText("3.0 GB").length).toBeGreaterThan(0);
    });
});
