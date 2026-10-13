// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const detailCmd = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { BlockStatsDetailCommand: (...args: unknown[]) => detailCmd(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const MB = 1024 * 1024;
vi.mock("@/app/hook/useBlockStats", () => ({
    useBlockStats: () => () => ({ cpu: 0.3, mem: 90 * MB, peak: 184 * MB, pids: 1 }),
}));

import type { BlockStatsDetail } from "@/app/store/rpc-api/blockstats";
import { BlockStatsBadge, describeConnection, describeProcesses, formatBytes, formatSince } from "./blockstats";

function detail(over: Partial<BlockStatsDetail> = {}): BlockStatsDetail {
    return {
        processes: 1,
        names: [{ name: "pwsh", count: 1 }],
        started_at_ms: new Date(2026, 9, 12, 13, 20).getTime(),
        mem_private: 10 * MB,
        mem_commit: 93 * MB,
        memory_metric: "private working set",
        ...over,
    };
}

afterEach(() => {
    cleanup();
    detailCmd.mockReset();
});

describe("the badge's tooltip", () => {
    it("asks for the processes only once it opens, and shows every figure", async () => {
        detailCmd.mockResolvedValue(detail());
        const { container } = render(() => <BlockStatsBadge blockId="b1" />);
        expect(detailCmd).not.toHaveBeenCalled();

        fireEvent.mouseEnter(container.querySelector(".block-stats-badge")!);
        await waitFor(() => expect(screen.getByText(/This pane's processes: pwsh \(1 process\)/)).toBeTruthy());
        expect(detailCmd).toHaveBeenCalledWith({}, { block_id: "b1" });
        for (const [label, value] of [
            ["CPU", "0.3%"],
            ["Memory", "90 MB"],
            ["Private", "10 MB"],
            ["Committed", "93 MB"],
            ["Peak", "184 MB"],
        ]) {
            expect(screen.getByText(label).nextElementSibling?.textContent).toBe(value);
        }
        expect(screen.getByText(/Windows trims it/)).toBeTruthy();
    });

    it("leaves out what the machine doesn't measure", async () => {
        detailCmd.mockResolvedValue(detail({ mem_commit: undefined, memory_metric: "anonymous resident + swap" }));
        const { container } = render(() => <BlockStatsBadge blockId="b1" />);
        fireEvent.mouseEnter(container.querySelector(".block-stats-badge")!);
        await waitFor(() => expect(screen.getByText("Private")).toBeTruthy());
        expect(screen.queryByText("Committed")).toBeNull();
        expect(screen.queryByText(/Windows trims it/)).toBeNull();
    });

    it("still shows the badge's own figures when the detail can't be read", async () => {
        detailCmd.mockRejectedValue(new Error("no process for this pane"));
        const { container } = render(() => <BlockStatsBadge blockId="b1" />);
        fireEvent.mouseEnter(container.querySelector(".block-stats-badge")!);
        await waitFor(() => expect(screen.getByText("This pane's processes (1 process)")).toBeTruthy());
        expect(screen.getByText("Memory")).toBeTruthy();
        expect(screen.queryByText("Private")).toBeNull();
    });
});

describe("a remote pane's tooltip", () => {
    it("leads with the machine and says the figures are this computer's ssh", async () => {
        detailCmd.mockResolvedValue(
            detail({
                names: [{ name: "ssh", count: 1 }],
                connection: { name: "build", kind: "ssh", host: "build.example.net", address: "203.0.113.5" },
            }),
        );
        const { container } = render(() => <BlockStatsBadge blockId="b1" />);
        fireEvent.mouseEnter(container.querySelector(".block-stats-badge")!);
        await waitFor(() =>
            expect(screen.getByText("Connected to build (build.example.net, 203.0.113.5)")).toBeTruthy(),
        );
        expect(screen.getByText(/Measured on this computer: ssh \(1 process\)/)).toBeTruthy();
        expect(screen.queryByText(/This pane's processes/)).toBeNull();
        expect(screen.queryByText(/The shell and everything started in it/)).toBeNull();
    });

    it("shows a local pane without a connection line", async () => {
        detailCmd.mockResolvedValue(detail());
        const { container } = render(() => <BlockStatsBadge blockId="b1" />);
        fireEvent.mouseEnter(container.querySelector(".block-stats-badge")!);
        await waitFor(() => expect(screen.getByText("Private")).toBeTruthy());
        expect(screen.queryByText(/Connected to/)).toBeNull();
    });
});

describe("formatting", () => {
    it("names a connection by what it says beyond its own name", () => {
        expect(describeConnection({ name: "build", kind: "ssh", host: "build.example.net", address: "203.0.113.5" })).toBe(
            "Connected to build (build.example.net, 203.0.113.5)",
        );
        // The host is the name already: only the address is new.
        expect(describeConnection({ name: "me@box.lan:2222", kind: "ssh", host: "box.lan", address: "192.0.2.9" })).toBe(
            "Connected to me@box.lan:2222 (192.0.2.9)",
        );
        expect(describeConnection({ name: "192.0.2.4", kind: "ssh", host: "192.0.2.4", address: "192.0.2.4" })).toBe(
            "Connected to 192.0.2.4",
        );
        expect(describeConnection({ name: "wsl://Ubuntu", kind: "wsl", host: "Ubuntu", address: "198.51.100.5" })).toBe(
            "Connected to WSL Ubuntu (198.51.100.5)",
        );
        expect(describeConnection({ name: "far", kind: "ssh" })).toBe("Connected to far");
    });


    it("names at most three executables and counts the rest", () => {
        const names = [
            { name: "pwsh", count: 1 },
            { name: "node", count: 3 },
            { name: "git", count: 1 },
            { name: "cargo", count: 1 },
            { name: "rustc", count: 2 },
        ];
        expect(describeProcesses(detail({ processes: 8, names }))).toBe("pwsh, node ×3, git and 3 more (8 processes)");
    });

    it("spells sizes in KB, MB and GB", () => {
        expect(formatBytes(512 * 1024)).toBe("512 KB");
        expect(formatBytes(90.4 * MB)).toBe("90 MB");
        expect(formatBytes(1.5 * 1024 * MB)).toBe("1.5 GB");
    });

    it("gives a time today and a date before", () => {
        const now = new Date(2026, 9, 12, 18, 0);
        expect(formatSince(new Date(2026, 9, 12, 13, 20).getTime(), now)).toMatch(/13:20|1:20/);
        expect(formatSince(new Date(2026, 9, 11, 13, 20).getTime(), now)).toMatch(/Oct 11/);
    });
});
