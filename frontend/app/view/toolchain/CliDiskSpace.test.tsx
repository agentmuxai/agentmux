// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const prune = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ToolchainPruneCommand: (_c: unknown, data: unknown) => prune(data) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { CliDiskSpace, formatBytes } from "./CliDiskSpace";

const item = (over: Record<string, unknown> = {}) => ({
    dir: "/h/shared/cli/claude/2.1.200",
    provider: "claude",
    version: "2.1.200",
    legacy: false,
    bytes: 210_000_000,
    idle_days: 61,
    ...over,
});
const result = (over: Record<string, unknown> = {}) => ({
    dry_run: true,
    scan_ok: true,
    candidates: [],
    reclaimable_bytes: 0,
    removed: [],
    skipped: [],
    kept: 3,
    max_idle_days: 30,
    ...over,
});

beforeEach(() => prune.mockReset());
afterEach(() => cleanup());

describe("formatBytes", () => {
    it("picks a readable unit", () => {
        expect(formatBytes(12)).toBe("12 B");
        expect(formatBytes(12_345)).toBe("12 KB");
        expect(formatBytes(340_000_000)).toBe("340 MB");
        expect(formatBytes(8_300_000_000)).toBe("8.3 GB");
    });
});

describe("CliDiskSpace", () => {
    it("deletes nothing until asked: only a Check button at first, and Check is a dry run", async () => {
        prune.mockResolvedValue(result());
        render(() => <CliDiskSpace />);
        expect(prune).not.toHaveBeenCalled();
        expect(screen.queryByRole("button", { name: /Remove/ })).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: /Check for old versions/ }));
        await waitFor(() => expect(prune).toHaveBeenCalledWith({ dry_run: true }));
    });

    it("says so when there is nothing to remove", async () => {
        prune.mockResolvedValue(result());
        render(() => <CliDiskSpace />);
        fireEvent.click(screen.getByRole("button", { name: /Check for old versions/ }));
        expect(await screen.findByText(/Nothing to remove/)).toBeInTheDocument();
    });

    it("lists what would go and what it frees, and offers Remove only then", async () => {
        prune.mockResolvedValue(result({ candidates: [item(), item({ version: "2.1.201", bytes: 190_000_000 })], reclaimable_bytes: 400_000_000 }));
        render(() => <CliDiskSpace />);
        fireEvent.click(screen.getByRole("button", { name: /Check for old versions/ }));
        expect(await screen.findByText(/2 old installs · 400 MB can be freed/)).toBeInTheDocument();
        expect(screen.getByText(/claude 2.1.200 · 210 MB · unused for 61 days/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /Remove 2 · free 400 MB/ })).toBeInTheDocument();
    });

    it("Remove is the only call that sends dry_run: false", async () => {
        prune.mockResolvedValueOnce(result({ candidates: [item()], reclaimable_bytes: 210_000_000 }));
        prune.mockResolvedValueOnce(result({ dry_run: false, candidates: [item()], removed: [item()], reclaimable_bytes: 210_000_000 }));
        render(() => <CliDiskSpace />);
        fireEvent.click(screen.getByRole("button", { name: /Check for old versions/ }));
        fireEvent.click(await screen.findByRole("button", { name: /Remove 1/ }));
        expect(await screen.findByText(/Removed 1 · freed 210 MB/)).toBeInTheDocument();
        // and it names exactly the directories the check listed
        expect(prune.mock.calls.map((c) => c[0])).toEqual([
            { dry_run: true },
            { dry_run: false, only: ["/h/shared/cli/claude/2.1.200"] },
        ]);
    });

    it("reports installs left in place at removal time", async () => {
        prune.mockResolvedValueOnce(result({ candidates: [item()], reclaimable_bytes: 1 }));
        prune.mockResolvedValueOnce(result({ dry_run: false, removed: [], skipped: [{ dir: "/x", reason: "install lock is held" }] }));
        render(() => <CliDiskSpace />);
        fireEvent.click(screen.getByRole("button", { name: /Check for old versions/ }));
        fireEvent.click(await screen.findByRole("button", { name: /Remove 1/ }));
        expect(await screen.findByText(/Left in place \(1\)/)).toBeInTheDocument();
    });

    it("explains a removal that did nothing because the scan failed at that moment", async () => {
        prune.mockResolvedValueOnce(result({ candidates: [item()], reclaimable_bytes: 210_000_000 }));
        prune.mockResolvedValueOnce(result({ dry_run: false, scan_ok: false }));
        render(() => <CliDiskSpace />);
        fireEvent.click(screen.getByRole("button", { name: /Check for old versions/ }));
        fireEvent.click(await screen.findByRole("button", { name: /Remove 1/ }));
        expect(await screen.findByText(/nothing was removed/)).toBeInTheDocument();
        expect(screen.queryByText(/Removed 0/)).toBeNull();
    });

    it("offers nothing when the running-process scan failed", async () => {
        prune.mockResolvedValue(result({ scan_ok: false }));
        render(() => <CliDiskSpace />);
        fireEvent.click(screen.getByRole("button", { name: /Check for old versions/ }));
        expect(await screen.findByText(/Couldn't tell which CLIs are running/)).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: /Remove/ })).toBeNull();
    });

    it("shows a failure and lets the user try again", async () => {
        prune.mockImplementationOnce(() => Promise.reject(new Error("srv down")));
        render(() => <CliDiskSpace />);
        fireEvent.click(screen.getByRole("button", { name: /Check for old versions/ }));
        expect(await screen.findByRole("alert")).toHaveTextContent("srv down");
        expect(screen.getByRole("button", { name: /Check for old versions/ })).toBeInTheDocument();
    });
});
