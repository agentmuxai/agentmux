// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const detailCmd = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { BlockStatsDetailCommand: (...args: unknown[]) => detailCmd(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const MB = 1024 * 1024;
vi.mock("@/app/hook/useBlockStats", () => ({
    useBlockStats: () => () => ({ cpu: 0.3, mem: 90 * MB, peak: 184 * MB, pids: 1 }),
}));

import type { BlockStatsDetail, BlockStatsFigures, BlockStatsRemote } from "@/app/store/rpc-api/blockstats";
import {
    BlockStatsBadge,
    describeConnection,
    describeProcesses,
    formatBytes,
    formatSince,
    isRemoteConnection,
    remoteBadge,
} from "./blockstats";

function figures(over: Partial<BlockStatsFigures> = {}): BlockStatsFigures {
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

const local = (over: Partial<BlockStatsFigures> = {}): BlockStatsDetail => ({ local: figures(over) });

const HOST = { cpu_count: 8, cpu: 300, mem: 6 * 1024 * MB, processes: 400, memory_metric: "anonymous resident + swap" };

function remote(r: Partial<BlockStatsRemote>): BlockStatsDetail {
    return {
        connection: { name: "build", kind: "ssh", host: "build.example.net", address: "203.0.113.5" },
        remote: { found: "session", ...r },
    };
}

const REMOTE_FIGURES = figures({
    names: [
        { name: "bash", count: 1 },
        { name: "node", count: 2 },
    ],
    processes: 3,
    cpu: 12.5,
    mem_resident: 90 * MB,
    mem_private: 40 * MB,
    mem_commit: undefined,
    memory_metric: "anonymous resident + swap",
});

afterEach(() => {
    cleanup();
    detailCmd.mockReset();
});

const hover = (container: HTMLElement) => fireEvent.mouseEnter(container.querySelector(".block-stats-badge")!);
const value = (label: string) => screen.getByText(label).nextElementSibling?.textContent;

describe("a local pane's tooltip", () => {
    it("asks for the processes only once it opens, and shows every figure", async () => {
        detailCmd.mockResolvedValue(local());
        const { container } = render(() => <BlockStatsBadge blockId="b1" />);
        expect(detailCmd).not.toHaveBeenCalled();

        hover(container);
        await waitFor(() => expect(screen.getByText(/This pane's processes: pwsh \(1 process\)/)).toBeTruthy());
        expect(detailCmd).toHaveBeenCalledWith({}, { block_id: "b1" });
        for (const [label, v] of [
            ["CPU", "0.3%"],
            ["Memory", "90 MB"],
            ["Private", "10 MB"],
            ["Committed", "93 MB"],
            ["Peak", "184 MB"],
        ]) {
            expect(value(label)).toBe(v);
        }
        expect(screen.getByText(/Windows trims it/)).toBeTruthy();
    });

    it("leaves out what the machine doesn't measure", async () => {
        detailCmd.mockResolvedValue(local({ mem_commit: undefined, memory_metric: "anonymous resident + swap" }));
        const { container } = render(() => <BlockStatsBadge blockId="b1" connection="local" />);
        hover(container);
        await waitFor(() => expect(screen.getByText("Private")).toBeTruthy());
        expect(screen.queryByText("Committed")).toBeNull();
        expect(screen.queryByText(/Windows trims it/)).toBeNull();
    });

    it("still shows the badge's own figures when the detail can't be read", async () => {
        detailCmd.mockRejectedValue(new Error("no process for this pane"));
        const { container } = render(() => <BlockStatsBadge blockId="b1" />);
        hover(container);
        await waitFor(() => expect(screen.getByText("This pane's processes (1 process)")).toBeTruthy());
        expect(screen.getByText("Memory")).toBeTruthy();
        expect(screen.queryByText("Private")).toBeNull();
    });
});

describe("a remote pane's badge", () => {
    // jsdom lays nothing out, so no element has an offsetParent: say the
    // pane is on screen unless a test hides it.
    let onScreen = true;
    beforeEach(() => {
        onScreen = true;
        Object.defineProperty(HTMLElement.prototype, "offsetParent", {
            configurable: true,
            get() {
                return onScreen ? document.body : null;
            },
        });
    });
    afterEach(() => {
        delete (HTMLElement.prototype as { offsetParent?: unknown }).offsetParent;
    });

    it("asks nothing while its pane isn't on screen, the first time included", async () => {
        onScreen = false;
        detailCmd.mockResolvedValue(remote({ figures: REMOTE_FIGURES, host: HOST }));
        const { container } = render(() => <BlockStatsBadge blockId="r1" connection="build" />);
        await new Promise((r) => setTimeout(r, 50));
        expect(detailCmd).not.toHaveBeenCalled();
        expect(container.querySelector(".block-stats-badge")).toBeNull();
    });

    it("asks its machine without being hovered, and shows the remote shell's figures", async () => {
        detailCmd.mockResolvedValue(remote({ figures: REMOTE_FIGURES, host: HOST, peak: 120 * MB }));
        const { container } = render(() => <BlockStatsBadge blockId="r1" connection="build" />);
        await waitFor(() => expect(container.querySelector(".block-stats-badge")?.textContent).toBe("12.5%90M"));
        expect(detailCmd).toHaveBeenCalledWith({}, { block_id: "r1" });

        hover(container);
        await waitFor(() =>
            expect(screen.getByText("Connected to build (build.example.net, 203.0.113.5)")).toBeTruthy()
        );
        expect(screen.getByText(/Measured on that machine: bash, node ×2 \(3 processes\)/)).toBeTruthy();
        expect(screen.getByText(/the pane's durable session/)).toBeTruthy();
        for (const [label, v] of [
            ["CPU", "12.5%"],
            ["Memory", "90 MB"],
            ["Private", "40 MB"],
            ["Peak", "120 MB"],
            ["Host CPU", "300.0%"],
            ["Host memory", "6.0 GB"],
        ]) {
            expect(value(label)).toBe(v);
        }
        expect(screen.queryByText("Committed")).toBeNull();
    });

    it("marks the host's own figures when the pane's can't be told apart", async () => {
        detailCmd.mockResolvedValue(remote({ found: "none", host: HOST }));
        const { container } = render(() => <BlockStatsBadge blockId="r1" connection="build" />);
        await waitFor(() => expect(container.querySelector(".block-stats-badge")?.textContent).toBe("host 300.0%6.0G"));
        expect(container.querySelector(".block-stats-badge.host")).toBeTruthy();
        hover(container);
        await waitFor(() => expect(screen.getByText(/can't tell this pane's processes from the rest/)).toBeTruthy());
        expect(screen.queryByText("Peak")).toBeNull();
    });

    it("says why when the machine couldn't be asked", async () => {
        detailCmd.mockResolvedValue(remote({ found: "unavailable", reason: "AgentMux's helper isn't on build" }));
        const { container } = render(() => <BlockStatsBadge blockId="r1" connection="build" />);
        await waitFor(() => expect(container.querySelector(".block-stats-badge")?.textContent).toBe("–"));
        hover(container);
        await waitFor(() => expect(screen.getByText("Not measured: AgentMux's helper isn't on build.")).toBeTruthy());
    });
});

describe("formatting", () => {
    it("names a connection by what it says beyond its own name", () => {
        expect(
            describeConnection({ name: "build", kind: "ssh", host: "build.example.net", address: "203.0.113.5" })
        ).toBe("Connected to build (build.example.net, 203.0.113.5)");
        // The host is the name already: only the address is new.
        expect(
            describeConnection({ name: "me@box.lan:2222", kind: "ssh", host: "box.lan", address: "192.0.2.9" })
        ).toBe("Connected to me@box.lan:2222 (192.0.2.9)");
        expect(describeConnection({ name: "192.0.2.4", kind: "ssh", host: "192.0.2.4", address: "192.0.2.4" })).toBe(
            "Connected to 192.0.2.4"
        );
        expect(describeConnection({ name: "wsl://Ubuntu", kind: "wsl", host: "Ubuntu", address: "198.51.100.5" })).toBe(
            "Connected to WSL Ubuntu (198.51.100.5)"
        );
        expect(describeConnection({ name: "far", kind: "ssh" })).toBe("Connected to far");
    });

    it("tells a remote connection from a local one", () => {
        expect(isRemoteConnection(undefined)).toBe(false);
        expect(isRemoteConnection("")).toBe(false);
        expect(isRemoteConnection("local")).toBe(false);
        expect(isRemoteConnection("me@box")).toBe(true);
        expect(isRemoteConnection("wsl://Ubuntu")).toBe(true);
    });

    it("gives the badge the pane's figures, else the host's, else nothing", () => {
        expect(remoteBadge({ found: "marker", figures: REMOTE_FIGURES, host: HOST })).toEqual({
            cpu: "12.5%",
            mem: "90M",
            host: false,
        });
        expect(remoteBadge({ found: "marker", figures: { ...REMOTE_FIGURES, cpu: undefined } })?.cpu).toBe("–%");
        expect(remoteBadge({ found: "marker", figures: { ...REMOTE_FIGURES, mem_resident: undefined } })?.mem).toBe(
            "–"
        );
        expect(remoteBadge({ found: "none", host: HOST })).toEqual({ cpu: "300.0%", mem: "6.0G", host: true });
        expect(remoteBadge({ found: "unavailable", reason: "x" })).toBeNull();
    });

    it("names at most three executables and counts the rest", () => {
        const names = [
            { name: "pwsh", count: 1 },
            { name: "node", count: 3 },
            { name: "git", count: 1 },
            { name: "cargo", count: 1 },
            { name: "rustc", count: 2 },
        ];
        expect(describeProcesses(figures({ processes: 8, names }))).toBe("pwsh, node ×3, git and 3 more (8 processes)");
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
