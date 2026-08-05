// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for the shared toolchain-capabilities store — the single point of
// entry that fixed the Docker "installed here, not installed there"
// divergence (docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md).

import { describe, test, expect, beforeEach, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ResolveCliCommand: vi.fn(),
        ContainerRuntimeAvailableCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { RpcApi } from "@/app/store/rpc-api";
import {
    ensureCapability,
    refreshCapability,
    getCapability,
    isAvailable,
    watchCapability,
    resetCapabilities,
} from "./toolchain-capabilities";

beforeEach(() => {
    vi.clearAllMocks();
    resetCapabilities();
});

describe("checkKind dispatch", () => {
    test("a liveness-kind tool (docker) calls ContainerRuntimeAvailableCommand, not ResolveCliCommand", async () => {
        vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: true } as any);

        const result = await ensureCapability("docker");

        expect(result.status).toBe("available");
        expect(RpcApi.ContainerRuntimeAvailableCommand).toHaveBeenCalledTimes(1);
        expect(RpcApi.ResolveCliCommand).not.toHaveBeenCalled();
        expect(isAvailable("docker")).toBe(true);
    });

    test("a path-kind tool (git) calls ResolveCliCommand, not ContainerRuntimeAvailableCommand", async () => {
        vi.mocked(RpcApi.ResolveCliCommand).mockResolvedValue({
            version: "2.44.0", cli_path: "/usr/bin/git", source: "path",
        } as any);

        const result = await ensureCapability("git");

        expect(result.status).toBe("available");
        expect(result.version).toBe("2.44.0");
        expect(RpcApi.ResolveCliCommand).toHaveBeenCalledTimes(1);
        expect(RpcApi.ContainerRuntimeAvailableCommand).not.toHaveBeenCalled();
    });

    test("docker CLI on PATH but daemon down reports unavailable (the bug this store exists to fix)", async () => {
        // A path-only check would have said "available" here — that's
        // exactly the false positive that caused the toolchain widget and
        // the create-agent modal to disagree.
        vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: false } as any);

        const result = await ensureCapability("docker");

        expect(result.status).toBe("unavailable");
        expect(isAvailable("docker")).toBe(false);
    });

    test("RPC rejection maps to unavailable, not a thrown error", async () => {
        vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockRejectedValue(new Error("timeout"));
        const result = await ensureCapability("docker");
        expect(result.status).toBe("unavailable");
    });
});

describe("concurrent-call de-duplication", () => {
    test("two concurrent ensureCapability calls for the same id share one RPC", async () => {
        let resolveRpc!: (v: any) => void;
        vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockReturnValue(
            new Promise((resolve) => { resolveRpc = resolve; }) as any,
        );

        const p1 = ensureCapability("docker");
        const p2 = ensureCapability("docker");
        resolveRpc({ available: true });
        const [r1, r2] = await Promise.all([p1, p2]);

        expect(RpcApi.ContainerRuntimeAvailableCommand).toHaveBeenCalledTimes(1);
        expect(r1.status).toBe("available");
        expect(r2.status).toBe("available");
    });

    test("a completed (non-forced) call is served from cache without a new RPC", async () => {
        vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: true } as any);
        await ensureCapability("docker");
        await ensureCapability("docker");
        expect(RpcApi.ContainerRuntimeAvailableCommand).toHaveBeenCalledTimes(1);
    });
});

describe("refreshCapability / force", () => {
    test("refreshCapability issues a fresh RPC even when a cached result exists", async () => {
        vi.mocked(RpcApi.ContainerRuntimeAvailableCommand)
            .mockResolvedValueOnce({ available: false } as any)
            .mockResolvedValueOnce({ available: true } as any);

        const first = await ensureCapability("docker");
        expect(first.status).toBe("unavailable");

        const second = await refreshCapability("docker");
        expect(second.status).toBe("available");
        expect(RpcApi.ContainerRuntimeAvailableCommand).toHaveBeenCalledTimes(2);
    });
});

describe("watchCapability", () => {
    test("polls at the given interval and the returned stop function clears it", async () => {
        vi.useFakeTimers();
        vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: true } as any);

        const stop = watchCapability("docker", 1000);
        await vi.advanceTimersByTimeAsync(0); // flush the initial probe
        expect(RpcApi.ContainerRuntimeAvailableCommand).toHaveBeenCalledTimes(1);

        await vi.advanceTimersByTimeAsync(1000);
        expect(RpcApi.ContainerRuntimeAvailableCommand).toHaveBeenCalledTimes(2);

        stop();
        await vi.advanceTimersByTimeAsync(5000);
        expect(RpcApi.ContainerRuntimeAvailableCommand).toHaveBeenCalledTimes(2);

        vi.useRealTimers();
    });
});

describe("unknown tool id", () => {
    test("getCapability defaults to unknown before any probe", () => {
        expect(getCapability("nonexistent-tool")).toEqual({ status: "unknown" });
        expect(isAvailable("nonexistent-tool")).toBe(false);
    });
});
